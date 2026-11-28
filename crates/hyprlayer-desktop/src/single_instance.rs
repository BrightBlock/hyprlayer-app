//! One client per profile: a lock file decides, and a local socket lets a second launch ask the
//! first to come forward. Named apart from the Tauri app's, which uses its own plugin.

use std::path::{Path, PathBuf};

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

use crate::io::Io;

pub enum Instance {
    First(Guard),
    /// Another client holds the profile and was asked to focus.
    Second,
}

pub struct Guard {
    _lock: std::fs::File,
    #[cfg(unix)]
    listener: std::os::unix::net::UnixListener,
}

/// A short private dir, since `sun_path` caps a socket path near 104 bytes. Not the server's
/// `socket::runtime_dir`: a server starting on this profile deletes every socket in that one.
#[cfg(unix)]
pub fn socket_dir(data: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let base = match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(xdg) if cfg!(target_os = "linux") => PathBuf::from(xdg).join("hyprlayer"),
        _ => std::env::temp_dir(),
    };
    let hash = hyprlayer_server::socket::profile_hash(data);
    let dir = base.join(format!("hl-native-{hash}"));
    let made = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)));
    match made {
        Ok(()) => dir,
        Err(_) => data.to_path_buf(),
    }
}

pub fn acquire(data: &Path) -> std::io::Result<Instance> {
    let lock = std::fs::File::create(data.join("native-instance.lock"))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            #[cfg(unix)]
            ask_to_focus(&socket_dir(data).join("native.sock"));
            return Ok(Instance::Second);
        }
        Err(std::fs::TryLockError::Error(e)) => return Err(e),
    }
    #[cfg(unix)]
    {
        // A quit skips destructors, so the last owner's socket may still be there.
        let path = socket_dir(data).join("native.sock");
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        Ok(Instance::First(Guard {
            _lock: lock,
            listener,
        }))
    }
    #[cfg(not(unix))]
    Ok(Instance::First(Guard { _lock: lock }))
}

#[cfg(unix)]
fn ask_to_focus(path: &Path) {
    use std::io::Write as _;
    // The owner binds right after it locks; a launch racing it waits briefly.
    for _ in 0..20 {
        if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(path) {
            let _ = stream.write_all(b"focus\n");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    log::warn!(
        "another client holds this profile but did not answer at {}",
        path.display()
    );
}

impl Guard {
    /// Yields once per later launch; the receiver lives as long as the app.
    pub fn serve(self, io: &Io) -> UnboundedReceiver<()> {
        let (tx, rx) = unbounded_channel();
        #[cfg(unix)]
        {
            let Guard { _lock, listener } = self;
            io.handle().spawn(async move {
                let _lock = _lock;
                let Ok(listener) = tokio::net::UnixListener::from_std(listener) else {
                    return;
                };
                while let Ok((_stream, _)) = listener.accept().await {
                    if tx.send(()).is_err() {
                        return;
                    }
                }
            });
        }
        #[cfg(not(unix))]
        {
            let _ = (io, tx);
            std::mem::forget(self);
        }
        rx
    }
}
