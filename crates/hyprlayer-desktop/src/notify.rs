//! Desktop notifications behind one blocking `send`: `UNUserNotificationCenter` on macOS,
//! notify-rust elsewhere. Call it off the foreground; the reply can take seconds.

pub fn send(title: &str, body: &str) -> Result<(), String> {
    imp::send(title, body)
}

#[cfg(target_os = "macos")]
mod imp {
    use std::sync::{mpsc, Once};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use block2::{DynBlock, RcBlock};
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{define_class, msg_send, AllocAnyThread};
    use objc2_foundation::{NSBundle, NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
        UNNotificationPresentationOptions, UNNotificationRequest, UNUserNotificationCenter,
        UNUserNotificationCenterDelegate,
    };

    const REPLY: Duration = Duration::from_secs(10);

    define_class!(
        // SAFETY: NSObject has no subclassing requirements and `Delegate` has no `Drop`.
        #[unsafe(super(NSObject))]
        #[name = "HyprlayerNotificationDelegate"]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            // Without this the system silences a notification while the app is frontmost.
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                handler: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                handler.call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List
                    | UNNotificationPresentationOptions::Sound,));
            }
        }
    );

    impl Delegate {
        fn new() -> Retained<Self> {
            let this = Self::alloc().set_ivars(());
            // SAFETY: NSObject's designated initializer.
            unsafe { msg_send![super(this), init] }
        }
    }

    fn describe(error: *mut NSError) -> String {
        // SAFETY: the center passes null or a live NSError for the handler's duration.
        match unsafe { error.as_ref() } {
            Some(error) => error.localizedDescription().to_string(),
            None => "notifications are not allowed for this app".to_string(),
        }
    }

    pub fn send(title: &str, body: &str) -> Result<(), String> {
        // An unbundled binary has no identity, and the center raises when one asks for it.
        if NSBundle::mainBundle().bundleIdentifier().is_none() {
            return Err("this process has no bundle identifier; run it inside an .app".into());
        }
        let center = UNUserNotificationCenter::currentNotificationCenter();
        static DELEGATE: Once = Once::new();
        DELEGATE.call_once(|| {
            let delegate = Delegate::new();
            center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            // The center holds its delegate weakly.
            std::mem::forget(delegate);
        });

        let (tx, rx) = mpsc::channel();
        let answered = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            let _ = tx.send(if granted.as_bool() {
                Ok(())
            } else {
                Err(describe(error))
            });
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &answered,
        );
        rx.recv_timeout(REPLY)
            .map_err(|_| "no answer to the authorization request".to_string())??;

        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let id = NSString::from_str(&format!("hyprlayer-{nanos}"));
        let request =
            UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
        let (tx, rx) = mpsc::channel();
        let added = RcBlock::new(move |error: *mut NSError| {
            let _ = tx.send(if error.is_null() {
                Ok(())
            } else {
                Err(describe(error))
            });
        });
        center.addNotificationRequest_withCompletionHandler(&request, Some(&added));
        rx.recv_timeout(REPLY)
            .map_err(|_| "the notification center did not answer".to_string())?
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn send(title: &str, body: &str) -> Result<(), String> {
        notify_rust::Notification::new()
            .appname("Hyprlayer")
            .summary(title)
            .body(body)
            .show()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
