//! Platform transport: a Unix domain socket on Unix, a named pipe on Windows.
//!
//! Both give the same two types — [`Listener`] and [`Stream`] — and [`Stream`]
//! implements `Read + Write`, so the framing in the parent module is written
//! once. The endpoint is a string in both worlds: a filesystem path on Unix,
//! `\\.\pipe\...` on Windows.

use std::io;

/// Where the daemon listens unless told otherwise.
///
/// Unix: `$XDG_RUNTIME_DIR/xlit/xlit.sock` — a per-user, mode-0700, tmpfs
/// directory the session manager cleans up on logout, which is exactly the
/// lifetime a socket wants. `$TMPDIR/xlit-$UID/xlit.sock` when there is no
/// runtime dir (a bare `ssh` session, say).
///
/// Windows: a named pipe. The pipe's ACL, not the path, is what keeps other
/// users out — see `Listener::bind`.
pub fn default_endpoint() -> String {
    if let Ok(explicit) = std::env::var("XLIT_SOCKET") {
        if !explicit.is_empty() {
            return explicit;
        }
    }
    platform_default_endpoint()
}

#[cfg(unix)]
mod imp {
    use std::io::{self, Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};

    pub fn platform_default_endpoint() -> String {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let uid = std::env::var("UID").unwrap_or_default();
                std::env::temp_dir().join(format!("xlit-{uid}"))
            })
            .join("xlit");
        dir.join("xlit.sock").to_string_lossy().into_owned()
    }

    /// A connected peer, from either end.
    pub struct Stream(UnixStream);

    impl Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Write for Stream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }

    pub struct Listener {
        inner: UnixListener,
        path: PathBuf,
    }

    impl Listener {
        /// Bind the socket, replacing a stale one left by a daemon that was
        /// killed rather than stopped.
        ///
        /// "Stale" is decided by connecting, not by the file's existence: a
        /// socket inode outlives its process, so `bind` on an existing path
        /// fails with `AddrInUse` whether or not anyone is listening. If a
        /// connect succeeds, another daemon really is up and we refuse.
        pub fn bind(endpoint: &str) -> io::Result<Self> {
            let path = PathBuf::from(endpoint);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
                restrict_to_owner(parent)?;
            }
            if path.exists() {
                if UnixStream::connect(&path).is_ok() {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        format!("another xlit-daemon is already listening on {endpoint}"),
                    ));
                }
                std::fs::remove_file(&path)?;
            }
            let inner = UnixListener::bind(&path)?;
            restrict_to_owner(&path)?;
            Ok(Listener { inner, path })
        }

        pub fn accept(&self) -> io::Result<Stream> {
            self.inner.accept().map(|(s, _)| Stream(s))
        }

        pub fn endpoint(&self) -> &str {
            // The path came from a `&str`, so it is still valid UTF-8.
            self.path.to_str().unwrap_or_default()
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            // Leaving the inode behind is harmless — `bind` reclaims it — but
            // tidying up keeps `$XDG_RUNTIME_DIR` honest.
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// `chmod 0700`. The socket lives in a per-user directory already; this is
    /// the belt to that braces, since anyone who can connect can read the
    /// user's dictionary picks and write to the learning store.
    fn restrict_to_owner(path: &Path) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
    }

    pub fn connect(endpoint: &str) -> io::Result<Stream> {
        UnixStream::connect(endpoint).map(Stream)
    }
}

#[cfg(windows)]
mod imp {
    use std::io::{self, Read, Write};

    use windows::core::HSTRING;
    use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_READ,
        FILE_GENERIC_WRITE, FILE_SHARE_MODE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };

    /// Per-user rather than machine-wide: two users logged into the same
    /// machine each get their own daemon and their own learning store.
    pub fn platform_default_endpoint() -> String {
        let user = std::env::var("USERNAME").unwrap_or_default();
        if user.is_empty() {
            r"\\.\pipe\xlit-daemon".to_string()
        } else {
            format!(r"\\.\pipe\xlit-daemon-{user}")
        }
    }

    /// Owns a pipe handle and closes it exactly once.
    pub struct Stream {
        handle: HANDLE,
        /// Server ends must be disconnected before they are closed, or the
        /// client sees the pipe stay open until the daemon exits.
        server: bool,
    }

    // A handle is just a number; the only thing that would make moving one
    // between threads unsafe is our own aliasing, and `Stream` is never cloned.
    unsafe impl Send for Stream {}

    impl Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            let mut read = 0u32;
            unsafe { ReadFile(self.handle, Some(buf), Some(&mut read), None) }
                .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
            Ok(read as usize)
        }
    }

    impl Write for Stream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let mut written = 0u32;
            unsafe { WriteFile(self.handle, Some(buf), Some(&mut written), None) }
                .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
            Ok(written as usize)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Drop for Stream {
        fn drop(&mut self) {
            unsafe {
                if self.server {
                    let _ = DisconnectNamedPipe(self.handle);
                }
                let _ = CloseHandle(self.handle);
            }
        }
    }

    pub struct Listener {
        endpoint: String,
    }

    impl Listener {
        /// Named pipes have no bind step: each `accept` creates an instance.
        /// Refuse to start if one already answers, so a second daemon does not
        /// silently take half the connections.
        pub fn bind(endpoint: &str) -> io::Result<Self> {
            if let Ok(existing) = connect(endpoint) {
                drop(existing);
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("another xlit-daemon is already listening on {endpoint}"),
                ));
            }
            Ok(Listener { endpoint: endpoint.to_string() })
        }

        pub fn accept(&self) -> io::Result<Stream> {
            let name = HSTRING::from(self.endpoint.as_str());
            // A default security descriptor gives the pipe the creator's token
            // as owner, which is what keeps other desktop users out.
            let handle = unsafe {
                CreateNamedPipeW(
                    &name,
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                    PIPE_UNLIMITED_INSTANCES,
                    8192,
                    8192,
                    0,
                    None,
                )
            };
            if handle.is_invalid() {
                return Err(io::Error::last_os_error());
            }
            let stream = Stream { handle, server: true };
            match unsafe { ConnectNamedPipe(handle, None) } {
                Ok(()) => Ok(stream),
                // The client beat us to it between create and connect; that is
                // a connected pipe, not a failure.
                Err(e) if e.code().0 as u32 & 0xFFFF == ERROR_PIPE_CONNECTED.0 => Ok(stream),
                Err(e) => Err(io::Error::from_raw_os_error(e.code().0)),
            }
        }

        pub fn endpoint(&self) -> &str {
            &self.endpoint
        }
    }

    pub fn connect(endpoint: &str) -> io::Result<Stream> {
        let name = HSTRING::from(endpoint);
        let handle = unsafe {
            CreateFileW(
                &name,
                (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }
        .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
        Ok(Stream { handle, server: false })
    }
}

use imp::platform_default_endpoint;
pub use imp::{connect, Listener, Stream};

/// True when a daemon answers at `endpoint` right now.
pub fn is_running(endpoint: &str) -> bool {
    match connect(endpoint) {
        Ok(mut s) => {
            crate::write_frame(&mut s, &crate::Request::Ping).is_ok()
                && matches!(
                    crate::read_frame::<_, crate::Response>(&mut s),
                    Ok(Some(r)) if r.ok
                )
        }
        Err(_) => false,
    }
}

/// Bind a listener at `endpoint`.
pub fn bind(endpoint: &str) -> io::Result<Listener> {
    Listener::bind(endpoint)
}
