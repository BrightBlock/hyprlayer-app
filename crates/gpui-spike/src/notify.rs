//! One `Notifier` over notify-rust; macOS may need a UNUserNotificationCenter path (see the record).
pub fn send(title: &str, body: &str) -> Result<(), String> {
    notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .show()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
