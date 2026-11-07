//! Unix-socket transport, shared by Linux and macOS. Sockets are mode 0600
//! in a 0700 directory (SPEC §10.4).

use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

use super::{Stream, TIMEOUT, Transport};

pub struct UnixSocket;

/// Create the endpoint's directory as 0700, or check that an existing one
/// belongs to us and nobody else can enter it.
fn private_dir(dir: &Path) -> io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let meta = std::fs::metadata(dir)?;
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    if meta.uid() != uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} belongs to another user", dir.display()),
        ));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

impl Transport for UnixSocket {
    type Listener = UnixListener;

    fn bind(endpoint: &Path) -> io::Result<UnixListener> {
        if let Some(dir) = endpoint.parent() {
            private_dir(dir)?;
        }
        let listener = UnixListener::bind(endpoint)?;
        std::fs::set_permissions(endpoint, std::fs::Permissions::from_mode(0o600))?;
        Ok(listener)
    }

    fn accept(listener: &UnixListener) -> io::Result<Box<dyn Stream>> {
        let (stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        Ok(Box::new(stream))
    }

    fn connect(endpoint: &Path) -> io::Result<Box<dyn Stream>> {
        let stream = UnixStream::connect(endpoint)?;
        // Long enough for the badge's loop to answer, short enough that a
        // hung badge doesn't hang the CLI.
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        Ok(Box::new(stream))
    }

    fn remove_stale(endpoint: &Path) -> io::Result<()> {
        match std::fs::remove_file(endpoint) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{self, Request, Response};

    #[test]
    fn serve_and_call_over_a_socket() {
        let dir = std::env::temp_dir().join(format!("pawprint-ipc-{}", std::process::id()));
        let endpoint = dir.join("test.sock");
        let _ = std::fs::remove_file(&endpoint);
        let listener = UnixSocket::bind(&endpoint).unwrap();
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&endpoint).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            UnixSocket::bind(&endpoint).unwrap_err().kind(),
            io::ErrorKind::AddrInUse,
            "a bound endpoint is the instance lock"
        );

        ipc::serve(listener, |incoming| {
            let response = match incoming.request {
                Request::Move => Response::error("no target display"),
                _ => Response::ok(),
            };
            incoming.reply(response);
            true
        });
        assert!(ipc::call(&endpoint, Request::Reload).unwrap().ok);
        let moved = ipc::call(&endpoint, Request::Move).unwrap();
        assert_eq!(moved.error.as_deref(), Some("no target display"));

        UnixSocket::remove_stale(&endpoint).unwrap();
        assert!(matches!(
            ipc::call(&endpoint, Request::Status),
            Err(ipc::CallError::NotRunning)
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
