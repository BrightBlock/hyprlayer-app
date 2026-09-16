//! Where the profile's socket lives and who may open it. Authorization is the
//! filesystem: a 0600 socket inside a 0700 directory, and nothing else.

use std::io;
use std::path::{Path, PathBuf};

/// `$XDG_RUNTIME_DIR/hyprlayer/<hash>.sock` on Linux, else
/// `<app_data_dir>/hyprlayer.sock`; a named pipe on Windows. The hash is of the
/// canonical `app_data_dir`, so a client that knows only the profile can derive
/// the name and two profiles never collide.
pub fn socket_path(app_data_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        return PathBuf::from(format!(
            r"\\.\pipe\hyprlayer-{}",
            profile_hash(app_data_dir)
        ));
    }
    if cfg!(target_os = "linux") {
        if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
            return PathBuf::from(dir)
                .join("hyprlayer")
                .join(format!("{}.sock", profile_hash(app_data_dir)));
        }
    }
    app_data_dir.join("hyprlayer.sock")
}

/// FNV-1a over the canonical path. Stable across builds, which `DefaultHasher`
/// is not: a newer client has to find an older server's socket.
pub fn profile_hash(app_data_dir: &Path) -> String {
    let canonical =
        std::fs::canonicalize(app_data_dir).unwrap_or_else(|_| app_data_dir.to_path_buf());
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in canonical.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")[..12].to_string()
}

#[cfg(unix)]
pub type Stream = tokio::net::UnixStream;
#[cfg(unix)]
pub type ClientStream = tokio::net::UnixStream;

#[cfg(unix)]
pub struct Listener {
    inner: tokio::net::UnixListener,
    path: PathBuf,
}

#[cfg(unix)]
impl Listener {
    /// Binds inside a 0700 directory and narrows the socket to 0600 before the
    /// first accept. A leftover socket file is unlinked only once a connect to
    /// it is refused, so a live server's socket is never stolen. Call from
    /// inside a Tokio runtime.
    pub fn bind(app_data_dir: &Path) -> io::Result<Self> {
        use std::os::unix::fs::PermissionsExt;

        let path = socket_path(app_data_dir);
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("socket path has no parent directory"))?;
        std::fs::create_dir_all(parent)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        remove_if_stale(&path)?;
        let inner = tokio::net::UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self { inner, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn accept(&mut self) -> io::Result<Stream> {
        self.inner.accept().await.map(|(stream, _)| stream)
    }
}

#[cfg(unix)]
fn remove_if_stale(path: &Path) -> io::Result<()> {
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!(
                "another hyprlayer-server is listening on {}",
                path.display()
            ),
        )),
        Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => std::fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(unix)]
impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
pub async fn connect(path: &Path) -> io::Result<ClientStream> {
    tokio::net::UnixStream::connect(path).await
}

#[cfg(windows)]
pub type Stream = tokio::net::windows::named_pipe::NamedPipeServer;
#[cfg(windows)]
pub type ClientStream = tokio::net::windows::named_pipe::NamedPipeClient;

#[cfg(windows)]
pub struct Listener {
    name: String,
    next: Option<Stream>,
}

#[cfg(windows)]
impl Listener {
    /// `first_pipe_instance` is the "no other listener" check: creating it fails
    /// while another server holds the name. The DACL is still the default; a
    /// current-user-only descriptor needs `windows-sys` and is not done yet.
    pub fn bind(app_data_dir: &Path) -> io::Result<Self> {
        use tokio::net::windows::named_pipe::ServerOptions;

        let name = socket_path(app_data_dir).to_string_lossy().into_owned();
        let first = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)?;
        Ok(Self {
            name,
            next: Some(first),
        })
    }

    pub fn path(&self) -> &Path {
        Path::new(&self.name)
    }

    pub async fn accept(&mut self) -> io::Result<Stream> {
        use tokio::net::windows::named_pipe::ServerOptions;

        let server = match self.next.take() {
            Some(server) => server,
            None => ServerOptions::new().create(&self.name)?,
        };
        server.connect().await?;
        self.next = Some(ServerOptions::new().create(&self.name)?);
        Ok(server)
    }
}

#[cfg(windows)]
pub async fn connect(path: &Path) -> io::Result<ClientStream> {
    use tokio::net::windows::named_pipe::ClientOptions;

    const ERROR_PIPE_BUSY: i32 = 231;
    let name = path.to_string_lossy().into_owned();
    let mut attempts = 0;
    loop {
        match ClientOptions::new().open(&name) {
            Ok(client) => return Ok(client),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && attempts < 50 => {
                attempts += 1;
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[tokio::test]
    async fn the_socket_is_0600_inside_a_0700_directory() {
        let dir = tempfile::tempdir().unwrap();
        let app_data_dir = dir.path().join("app-data");
        std::fs::create_dir_all(&app_data_dir).unwrap();

        let listener = Listener::bind(&app_data_dir).expect("bind");
        assert_eq!(mode(listener.path()), 0o600);
        assert_eq!(mode(listener.path().parent().unwrap()), 0o700);
    }

    #[tokio::test]
    async fn a_live_socket_is_refused_and_a_stale_one_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let app_data_dir = dir.path().join("app-data");
        std::fs::create_dir_all(&app_data_dir).unwrap();

        let live = Listener::bind(&app_data_dir).expect("first bind");
        let path = live.path().to_path_buf();
        let refused = Listener::bind(&app_data_dir)
            .err()
            .expect("second bind is refused");
        assert_eq!(refused.kind(), io::ErrorKind::AddrInUse);
        assert!(
            path.exists(),
            "a refused bind must not unlink the live socket"
        );
        drop(live);

        // A std listener dropped without unlinking is what a crashed server leaves.
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        let rebound = Listener::bind(&app_data_dir).expect("stale socket replaced");
        assert_eq!(rebound.path(), path);
    }

    #[test]
    fn the_profile_hash_is_stable_and_distinct_per_profile() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        assert_eq!(profile_hash(a.path()), profile_hash(a.path()));
        assert_ne!(profile_hash(a.path()), profile_hash(b.path()));
        assert_eq!(profile_hash(a.path()).len(), 12);
    }
}
