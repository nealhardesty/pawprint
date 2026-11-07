//! Unix-socket transport, shared by Linux and macOS. Sockets are mode 0600
//! in a 0700 directory (SPEC §10.4).
//!
//! The instance lock is an exclusive `flock` on `<key>.lock` next to the
//! socket, held for the life of the process. The kernel drops it when the
//! process dies, so a badge that stops answering (stopped, or stuck) still
//! owns its session, and a crashed one never does.

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

use super::{Stream, TIMEOUT, Transport};

pub struct UnixSocket;

/// The bound socket and the instance lock that makes it ours.
#[derive(Debug)]
pub struct Listener {
    socket: UnixListener,
    _lock: File,
}

/// Create the endpoint's directory as 0700, or check that an existing one
/// is a real directory (not a symlink) that belongs to us and nobody else
/// can enter.
fn private_dir(dir: &Path) -> io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not a directory (a symlink?)", dir.display()),
        ));
    }
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

/// Take the instance lock for `endpoint`, or fail with `AddrInUse` while
/// another live process holds it.
fn lock(endpoint: &Path) -> io::Result<File> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(endpoint.with_extension("lock"))?;
    // SAFETY: flock only reads the descriptor, which `file` keeps open.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let e = io::Error::last_os_error();
        return Err(if e.kind() == io::ErrorKind::WouldBlock {
            io::Error::new(io::ErrorKind::AddrInUse, "another badge holds the lock")
        } else {
            e
        });
    }
    Ok(file)
}

impl Transport for UnixSocket {
    type Listener = Listener;

    /// With the lock held, a socket left at the path belongs to a badge
    /// that died, so it is replaced.
    fn bind(endpoint: &Path) -> io::Result<Listener> {
        if let Some(dir) = endpoint.parent() {
            private_dir(dir)?;
        }
        let lock = lock(endpoint)?;
        Self::remove_stale(endpoint)?;
        let socket = UnixListener::bind(endpoint)?;
        std::fs::set_permissions(endpoint, std::fs::Permissions::from_mode(0o600))?;
        Ok(Listener {
            socket,
            _lock: lock,
        })
    }

    fn accept(listener: &Listener) -> io::Result<Box<dyn Stream>> {
        let (stream, _) = listener.socket.accept()?;
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

    #[test]
    fn the_lock_outlives_silence_and_dies_with_the_holder() {
        let dir = std::env::temp_dir().join(format!("pawprint-lock-{}", std::process::id()));
        let endpoint = dir.join("test.sock");
        let first = UnixSocket::bind(&endpoint).unwrap();
        // Nobody answers on `first` (nothing accepts), yet it keeps the endpoint.
        assert_eq!(
            UnixSocket::bind(&endpoint).unwrap_err().kind(),
            io::ErrorKind::AddrInUse
        );
        // A crash: the lock goes with the process, the socket file stays.
        drop(first);
        assert!(endpoint.exists());
        let _second = UnixSocket::bind(&endpoint).expect("a dead badge's socket is replaced");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refuses_a_symlinked_directory() {
        let dir = std::env::temp_dir().join(format!("pawprint-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("real")).unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join("link")).unwrap();
        let err = UnixSocket::bind(&dir.join("link").join("test.sock")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{err}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
