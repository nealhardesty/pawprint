//! Named-pipe transport for Windows (SPEC §10.4): `\\.\pipe\pawprint-<user>-<sid>`.
//!
//! The first instance is created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so a
//! second badge in the same session gets `AddrInUse`: the pipe is the
//! instance lock. The server always keeps one instance waiting before it
//! hands a connected one to the IPC thread, so the name never disappears
//! while the badge runs, and it disappears by itself when the badge dies;
//! `remove_stale` has nothing to do.
//!
//! Access: the pipe gets the default DACL, which gives full control to the
//! creating user, SYSTEM, and administrators, and read-only access to
//! everyone else. A request has to be written, so only those three can send
//! one. `PIPE_REJECT_REMOTE_CLIENTS` refuses connections over the network.

use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::time::Instant;

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    WaitNamedPipeW,
};

use super::{Stream, TIMEOUT, Transport};

pub struct NamedPipe;

/// The server side: the pipe name and the instance waiting for a client.
#[derive(Debug)]
pub struct PipeListener {
    name: Vec<u16>,
    waiting: RefCell<OwnedHandle>,
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

fn os_error(e: &io::Error, code: u32) -> bool {
    e.raw_os_error() == Some(code as i32)
}

/// One server instance of the pipe. Only the first may create the name.
fn instance(name: &[u16], first: bool) -> io::Result<OwnedHandle> {
    let open = PIPE_ACCESS_DUPLEX
        | if first {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
    let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
    // SAFETY: `name` is NUL-terminated; a null security descriptor means the
    // default DACL.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open,
            mode,
            PIPE_UNLIMITED_INSTANCES,
            4096,
            4096,
            0,
            std::ptr::null(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let e = io::Error::last_os_error();
        // FIRST_PIPE_INSTANCE fails with "access denied" when the name exists.
        if first && os_error(&e, ERROR_ACCESS_DENIED) {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "another badge holds this pipe",
            ));
        }
        return Err(e);
    }
    // SAFETY: a valid handle that nothing else owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

/// A connected server end. `flush` waits until the client has read
/// everything, so closing the handle afterwards never drops the answer.
struct ServerEnd(File);

impl Read for ServerEnd {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for ServerEnd {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        // FlushFileBuffers on a pipe blocks until the other end has read it.
        self.0.sync_all()
    }
}

impl Transport for NamedPipe {
    type Listener = PipeListener;

    fn bind(endpoint: &Path) -> io::Result<PipeListener> {
        let name = wide(endpoint);
        let first = instance(&name, true)?;
        Ok(PipeListener {
            name,
            waiting: RefCell::new(first),
        })
    }

    /// Pipe reads have no timeout. Only this user can write to the pipe,
    /// and the CLI always sends its line at once.
    fn accept(listener: &PipeListener) -> io::Result<Box<dyn Stream>> {
        let handle = listener.waiting.borrow().as_raw_handle();
        // SAFETY: a valid pipe handle that the listener keeps open;
        // synchronous (no OVERLAPPED). Disconnecting resets the instance for
        // the next client.
        let reset = |e| {
            unsafe { DisconnectNamedPipe(handle) };
            Err(e)
        };
        if unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) } == 0 {
            let e = io::Error::last_os_error();
            // A client that connected before we waited is still connected.
            if !os_error(&e, ERROR_PIPE_CONNECTED) {
                return reset(e);
            }
        }
        // The connected instance keeps the name alive while the next one is
        // created; a client arriving in between waits in `connect`.
        let next = match instance(&listener.name, false) {
            Ok(next) => next,
            Err(e) => return reset(e),
        };
        let connected = listener.waiting.replace(next);
        Ok(Box::new(ServerEnd(File::from(connected))))
    }

    fn connect(endpoint: &Path) -> io::Result<Box<dyn Stream>> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match OpenOptions::new().read(true).write(true).open(endpoint) {
                Ok(file) => return Ok(Box::new(file)),
                // Every instance is busy for a moment: wait for a free one.
                Err(e) if os_error(&e, ERROR_PIPE_BUSY) && Instant::now() < deadline => {
                    // SAFETY: the name is NUL-terminated.
                    unsafe { WaitNamedPipeW(wide(endpoint).as_ptr(), 500) };
                }
                // No such pipe is NotFound: no badge in this session.
                Err(e) => return Err(e),
            }
        }
    }

    fn remove_stale(_endpoint: &Path) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{self, Request, Response};

    #[test]
    fn serve_and_call_over_a_pipe() {
        let endpoint =
            std::path::PathBuf::from(format!(r"\\.\pipe\pawprint-test-{}", std::process::id()));
        assert!(matches!(
            ipc::call(&endpoint, Request::Status),
            Err(ipc::CallError::NotRunning)
        ));
        let listener = NamedPipe::bind(&endpoint).unwrap();
        assert_eq!(
            NamedPipe::bind(&endpoint).unwrap_err().kind(),
            io::ErrorKind::AddrInUse,
            "a bound pipe is the instance lock"
        );

        ipc::serve(listener, |incoming| {
            let response = match incoming.request {
                Request::Move => Response::error("no target display"),
                _ => Response::ok(),
            };
            incoming.reply(response);
            true
        });
        for _ in 0..3 {
            assert!(ipc::call(&endpoint, Request::Reload).unwrap().ok);
        }
        let moved = ipc::call(&endpoint, Request::Move).unwrap();
        assert_eq!(moved.error.as_deref(), Some("no target display"));
        assert_eq!(
            NamedPipe::bind(&endpoint).unwrap_err().kind(),
            io::ErrorKind::AddrInUse,
            "still locked after serving clients"
        );
    }
}
