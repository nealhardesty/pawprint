//! Named-pipe transport for Windows (SPEC §10.4): `\\.\pipe\pawprint-<user>-<sid>`.
//!
//! The first instance is created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so a
//! second badge in the same session gets `AddrInUse`: the pipe is the
//! instance lock. The server always keeps one instance waiting before it
//! hands a connected one to the IPC thread, so the name never disappears
//! while the badge runs, and it disappears by itself when the badge dies;
//! `remove_stale` has nothing to do.
//!
//! Access: every instance gets an explicit security descriptor, owned by
//! this user, whose protected DACL grants this user and SYSTEM and nobody
//! else (`O:<user>D:P(A;;GA;;;<user>)(A;;GA;;;SY)`), so no other account,
//! anonymous included, can open the pipe or add an instance to it.
//! `PIPE_REJECT_REMOTE_CLIENTS` refuses connections over the network.
//!
//! A client only talks to a pipe this user owns, which catches another
//! account creating the name first, and connects at `SECURITY_IDENTIFICATION`
//! so the server can't act as it. All I/O is overlapped and gives up after
//! [`TIMEOUT`], on both sides, so a client that never writes (or never
//! reads) can't stall the IPC thread, and a hung badge can't hang the CLI.

use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::time::Instant;

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, ERROR_SUCCESS, GENERIC_ALL, GENERIC_READ, GENERIC_WRITE, HANDLE,
    INVALID_HANDLE_VALUE, LocalFree, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_KERNEL_OBJECT};
use windows_sys::Win32::Security::{
    ACL, ACL_REVISION, AddAccessAllowedAce, CreateWellKnownSid, EqualSid, GetTokenInformation,
    InitializeAcl, InitializeSecurityDescriptor, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    PSID, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR,
    SetSecurityDescriptorControl, SetSecurityDescriptorDacl, SetSecurityDescriptorOwner,
    TOKEN_QUERY, TOKEN_USER, TokenUser, WinLocalSystemSid,
};

use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX, ReadFile, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows_sys::Win32::System::IO::{
    CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    WaitNamedPipeW,
};
use windows_sys::Win32::System::Threading::{CreateEventW, GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;

use super::{Stream, TIMEOUT, Transport};

pub struct NamedPipe;

/// winnt.h's `SECURITY_DESCRIPTOR_REVISION` (not worth a windows-sys feature).
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;

/// The server side: the pipe name and the instance waiting for a client.
#[derive(Debug)]
pub struct PipeListener {
    name: Vec<u16>,
    waiting: Mutex<Pipe>,
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

fn os_error(e: &io::Error, code: u32) -> bool {
    e.raw_os_error() == Some(code as i32)
}

fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a valid handle that nothing else owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

/// This process's user, as `GetTokenInformation(TokenUser)` gives it.
struct User {
    /// A `TOKEN_USER` and the SID it points into; u64 for alignment.
    buf: Vec<u64>,
}

impl User {
    fn current() -> io::Result<User> {
        let mut token = null_mut();
        // SAFETY: the pseudo-handle needs no closing; `token` is written.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = owned(token)?;
        let mut buf = vec![0u64; 16];
        loop {
            let mut len = 0;
            let size = (buf.len() * 8) as u32;
            // SAFETY: `buf` holds `size` bytes.
            let ok = unsafe {
                GetTokenInformation(
                    token.as_raw_handle(),
                    TokenUser,
                    buf.as_mut_ptr().cast(),
                    size,
                    &mut len,
                )
            };
            if ok != 0 {
                return Ok(User { buf });
            }
            if len <= size {
                return Err(io::Error::last_os_error());
            }
            buf.resize((len as usize).div_ceil(8), 0);
        }
    }

    fn sid(&self) -> PSID {
        // SAFETY: GetTokenInformation filled `buf` with a TOKEN_USER.
        unsafe { (*self.buf.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

/// A `LocalAlloc`'d security descriptor.
struct Descriptor(PSECURITY_DESCRIPTOR);

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: allocated by the system with LocalAlloc.
        unsafe { LocalFree(self.0) };
    }
}

/// The pipe's security descriptor: owned by this user, and a protected
/// DACL that grants this user and SYSTEM `GENERIC_ALL` and nobody else
/// (`O:<user>D:P(A;;GA;;;<user>)(A;;GA;;;SY)`). Built with the plain ACL
/// calls rather than from SDDL: parsing SDDL asks LSA about domain aliases,
/// which leaves RPC threads behind that wake the idle badge.
struct PrivateDescriptor {
    // The descriptor points into these heap buffers, which never move.
    _user: User,
    _system: Vec<u64>,
    _acl: Vec<u64>,
    sd: Box<SECURITY_DESCRIPTOR>,
}

impl PrivateDescriptor {
    fn new() -> io::Result<PrivateDescriptor> {
        let user = User::current()?;
        let fail = |ok: i32| {
            if ok == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        };
        // SECURITY_MAX_SID_SIZE is 68 bytes; two ACEs of at most 12 + 68.
        let mut system = vec![0u64; 9];
        let mut acl = vec![0u64; 24];
        let mut sd = Box::new(SECURITY_DESCRIPTOR::default());
        let sd_ptr: PSECURITY_DESCRIPTOR = (&raw mut *sd).cast();
        let acl_ptr = acl.as_mut_ptr().cast::<ACL>();
        let mut len = (system.len() * 8) as u32;
        // SAFETY: every buffer is as large as the call is told; the
        // descriptor keeps pointers into `user`, `system`, and `acl`,
        // which live as long as it does.
        unsafe {
            fail(CreateWellKnownSid(
                WinLocalSystemSid,
                null_mut(),
                system.as_mut_ptr().cast(),
                &mut len,
            ))?;
            fail(InitializeAcl(acl_ptr, (acl.len() * 8) as u32, ACL_REVISION))?;
            for sid in [user.sid(), system.as_mut_ptr().cast()] {
                fail(AddAccessAllowedAce(acl_ptr, ACL_REVISION, GENERIC_ALL, sid))?;
            }
            fail(InitializeSecurityDescriptor(
                sd_ptr,
                SECURITY_DESCRIPTOR_REVISION,
            ))?;
            fail(SetSecurityDescriptorOwner(sd_ptr, user.sid(), 0))?;
            fail(SetSecurityDescriptorDacl(sd_ptr, 1, acl_ptr, 0))?;
            fail(SetSecurityDescriptorControl(
                sd_ptr,
                SE_DACL_PROTECTED,
                SE_DACL_PROTECTED,
            ))?;
        }
        Ok(PrivateDescriptor {
            _user: user,
            _system: system,
            _acl: acl,
            sd,
        })
    }

    fn as_ptr(&self) -> PSECURITY_DESCRIPTOR {
        (&raw const *self.sd).cast_mut().cast()
    }
}

/// One end of the pipe, in overlapped mode, with the event its I/O waits on.
#[derive(Debug)]
struct Pipe {
    handle: OwnedHandle,
    event: OwnedHandle,
    /// The server end: `flush` waits for the client to finish reading.
    server: bool,
}

impl Pipe {
    fn new(handle: OwnedHandle, server: bool) -> io::Result<Pipe> {
        // SAFETY: an unnamed manual-reset event.
        let event = owned(unsafe { CreateEventW(null(), 1, 0, null()) })?;
        Ok(Pipe {
            handle,
            event,
            server,
        })
    }

    fn raw(&self) -> HANDLE {
        self.handle.as_raw_handle()
    }

    /// Start an overlapped operation and wait for it, at most `timeout_ms`.
    /// A timed-out operation is cancelled before the OVERLAPPED goes away.
    fn wait(
        &self,
        timeout_ms: u32,
        start: impl FnOnce(*mut OVERLAPPED) -> i32,
    ) -> io::Result<usize> {
        let mut ov = OVERLAPPED {
            hEvent: self.event.as_raw_handle(),
            ..Default::default()
        };
        if start(&mut ov) == 0 {
            let e = io::Error::last_os_error();
            if !os_error(&e, ERROR_IO_PENDING) {
                return Err(e);
            }
        }
        let mut n = 0;
        // SAFETY: `ov` stays in place until the operation is complete:
        // finished, failed, or cancelled and then waited for.
        unsafe {
            if GetOverlappedResultEx(self.raw(), &ov, &mut n, timeout_ms, 0) != 0 {
                return Ok(n as usize);
            }
            let e = io::Error::last_os_error();
            if os_error(&e, WAIT_TIMEOUT) {
                CancelIoEx(self.raw(), &ov);
                GetOverlappedResult(self.raw(), &ov, &mut n, 1);
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the other end of the pipe did not answer in time",
                ));
            }
            Err(e)
        }
    }

    fn timeout_ms() -> u32 {
        TIMEOUT.as_millis() as u32
    }
}

impl Read for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = buf.len().min(u32::MAX as usize) as u32;
        let ptr = buf.as_mut_ptr();
        // SAFETY: `buf` outlives the wait.
        match self.wait(Pipe::timeout_ms(), |ov| unsafe {
            ReadFile(self.raw(), ptr, len, null_mut(), ov)
        }) {
            // The other end closed: end of stream.
            Err(e) if os_error(&e, ERROR_BROKEN_PIPE) => Ok(0),
            r => r,
        }
    }
}

impl Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let len = buf.len().min(u32::MAX as usize) as u32;
        // SAFETY: `buf` outlives the wait.
        self.wait(Pipe::timeout_ms(), |ov| unsafe {
            WriteFile(self.raw(), buf.as_ptr(), len, null_mut(), ov)
        })
    }

    /// On the server end, wait (at most [`TIMEOUT`]) until the client has
    /// read the answer and closed its end, so closing ours never drops it.
    fn flush(&mut self) -> io::Result<()> {
        if self.server {
            let mut rest = [0u8; 64];
            while self.read(&mut rest)? != 0 {}
        }
        Ok(())
    }
}

/// One server instance of the pipe. Only the first may create the name.
fn instance(name: &[u16], first: bool) -> io::Result<Pipe> {
    let open = PIPE_ACCESS_DUPLEX
        | FILE_FLAG_OVERLAPPED
        | if first {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
    let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
    let sd = PrivateDescriptor::new()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.as_ptr(),
        bInheritHandle: 0,
    };
    // SAFETY: `name` is NUL-terminated; `attributes` and its descriptor
    // outlive the call.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open,
            mode,
            PIPE_UNLIMITED_INSTANCES,
            4096,
            4096,
            0,
            &attributes,
        )
    };
    match owned(handle) {
        // FIRST_PIPE_INSTANCE fails with "access denied" when the name exists.
        Err(e) if first && os_error(&e, ERROR_ACCESS_DENIED) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "another badge holds this pipe",
        )),
        Err(e) => Err(e),
        Ok(handle) => Pipe::new(handle, true),
    }
}

/// Fail unless the pipe behind `handle` is owned by this user: another
/// account may have created the name first to read our requests.
fn check_owner(handle: HANDLE) -> io::Result<()> {
    let user = User::current()?;
    let (mut owner, mut sd) = (null_mut(), null_mut());
    // SAFETY: `owner` points into `sd`, which is LocalAlloc'd and freed
    // when `_sd` drops.
    let code = unsafe {
        GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut sd,
        )
    };
    if code != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(code as i32));
    }
    let _sd = Descriptor(sd);
    // SAFETY: two valid SIDs.
    if unsafe { EqualSid(owner, user.sid()) } == 0 {
        return Err(another_user());
    }
    Ok(())
}

fn another_user() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "the pipe belongs to another user (not a PAWPRINT badge of yours)",
    )
}

impl Transport for NamedPipe {
    type Listener = PipeListener;

    fn bind(endpoint: &Path) -> io::Result<PipeListener> {
        let name = wide(endpoint);
        let first = instance(&name, true)?;
        Ok(PipeListener {
            name,
            waiting: Mutex::new(first),
        })
    }

    /// Wait for the next client; its reads and writes then time out.
    fn accept(listener: &PipeListener) -> io::Result<Box<dyn Stream>> {
        let mut waiting = listener.waiting.lock().unwrap_or_else(|e| e.into_inner());
        // A client that connected before we waited is already connected.
        let connected = waiting.wait(u32::MAX, |ov| {
            // SAFETY: a valid pipe handle; `ov` stays alive in `wait`.
            unsafe { ConnectNamedPipe(waiting.raw(), ov) }
        });
        if let Err(e) = connected
            && !os_error(&e, ERROR_PIPE_CONNECTED)
        {
            // Start over with a fresh instance for the next client.
            *waiting = instance(&listener.name, false)?;
            return Err(e);
        }
        // The connected instance keeps the name alive while the next one is
        // created; a client arriving in between waits in `connect`.
        let next = instance(&listener.name, false)?;
        Ok(Box::new(std::mem::replace(&mut *waiting, next)))
    }

    fn connect(endpoint: &Path) -> io::Result<Box<dyn Stream>> {
        let name = wide(endpoint);
        let deadline = Instant::now() + TIMEOUT;
        let handle = loop {
            // SAFETY: `name` is NUL-terminated.
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                    null_mut(),
                )
            };
            match owned(handle) {
                Ok(h) => break h,
                // Every instance is busy for a moment: wait for a free one.
                Err(e) if os_error(&e, ERROR_PIPE_BUSY) && Instant::now() < deadline => {
                    // SAFETY: the name is NUL-terminated.
                    unsafe { WaitNamedPipeW(name.as_ptr(), 500) };
                }
                // Only another user's pipe refuses us.
                Err(e) if os_error(&e, ERROR_ACCESS_DENIED) => return Err(another_user()),
                // No such pipe is NotFound: no badge in this session.
                Err(e) => return Err(e),
            }
        };
        check_owner(handle.as_raw_handle())?;
        // Let the badge take the foreground from us: `pawprint move` needs
        // it for the keyboard (SPEC §11.1.4). Fails harmlessly when we
        // aren't in the foreground ourselves.
        let mut pid = 0;
        // SAFETY: a valid client pipe handle.
        if unsafe { GetNamedPipeServerProcessId(handle.as_raw_handle(), &mut pid) } != 0 {
            unsafe { AllowSetForegroundWindow(pid) };
        }
        Ok(Box::new(Pipe::new(handle, false)?))
    }

    fn remove_stale(_endpoint: &Path) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{self, CallError, Request, Response};
    use std::time::Duration;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW,
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;

    fn endpoint(tag: &str) -> std::path::PathBuf {
        format!(r"\\.\pipe\pawprint-test-{tag}-{}", std::process::id()).into()
    }

    fn serve(listener: PipeListener) {
        ipc::serve(listener, |incoming| {
            let response = match incoming.request {
                Request::Move => Response::error("no target display"),
                _ => Response::ok(),
            };
            incoming.reply(response);
            true
        });
    }

    /// A raw client of `endpoint`, as anyone allowed to open it could make.
    fn raw_client(endpoint: &Path) -> std::fs::File {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(endpoint)
            .unwrap()
    }

    #[test]
    fn serve_and_call_over_a_pipe() {
        let endpoint = endpoint("serve");
        assert!(matches!(
            ipc::call(&endpoint, Request::Status),
            Err(CallError::NotRunning)
        ));
        let listener = NamedPipe::bind(&endpoint).unwrap();
        assert_eq!(
            NamedPipe::bind(&endpoint).unwrap_err().kind(),
            io::ErrorKind::AddrInUse,
            "a bound pipe is the instance lock"
        );
        serve(listener);
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

    const OWNER_AND_DACL: u32 = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;

    /// `sd`'s owner and DACL as SDDL, with the system's aliases (`LA`, `BA`).
    fn to_sddl(sd: PSECURITY_DESCRIPTOR) -> String {
        let mut text = null_mut();
        // SAFETY: a valid descriptor; the string is LocalAlloc'd and freed.
        unsafe {
            assert_ne!(
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    sd,
                    SDDL_REVISION_1,
                    OWNER_AND_DACL,
                    &mut text,
                    null_mut()
                ),
                0
            );
            let len = (0..).take_while(|&i| *text.add(i) != 0).count();
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
            LocalFree(text.cast());
            s
        }
    }

    /// Only this user and SYSTEM may open the pipe, and this user owns it.
    #[test]
    fn the_pipe_is_private() {
        let endpoint = endpoint("dacl");
        let _listener = NamedPipe::bind(&endpoint).unwrap();
        let client = raw_client(&endpoint);
        let mut sd = null_mut();
        // SAFETY: as in `check_owner`.
        let code = unsafe {
            GetSecurityInfo(
                client.as_raw_handle(),
                SE_KERNEL_OBJECT,
                OWNER_AND_DACL,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                &mut sd,
            )
        };
        assert_eq!(code, ERROR_SUCCESS);
        let actual = Descriptor(sd);
        // What we asked for, written the way the system writes it back
        // (generic access mapped to the pipe's FA).
        let want = PrivateDescriptor::new().unwrap();
        let want = to_sddl(want.as_ptr());
        println!("pipe descriptor {want}");
        assert_eq!(
            to_sddl(actual.0),
            want.replace(";GA;", ";FA;"),
            "owner and DACL"
        );
        let user = to_sddl(actual.0);
        let owner = &user[2..user.find("D:").unwrap()];
        assert_eq!(
            user,
            format!("O:{owner}D:P(A;;FA;;;{owner})(A;;FA;;;SY)"),
            "this user and SYSTEM only"
        );
    }

    /// A client that connects and never writes can't stall the badge: the
    /// server gives up on it after TIMEOUT and answers the next one.
    #[test]
    fn a_silent_client_does_not_stall_the_server() {
        let endpoint = endpoint("silent");
        serve(NamedPipe::bind(&endpoint).unwrap());
        let mut silent = raw_client(&endpoint);
        let start = Instant::now();
        let answered = loop {
            match ipc::call(&endpoint, Request::Status) {
                Ok(r) => break r,
                Err(e) => {
                    println!("while the silent client waits: {e}");
                    assert!(start.elapsed() < 3 * TIMEOUT, "the server stalled");
                }
            }
        };
        assert!(answered.ok);
        assert!(start.elapsed() < 3 * TIMEOUT);
        // The server closed the silent client's instance.
        let mut buf = [0u8; 8];
        assert_eq!(silent.read(&mut buf).unwrap_or(0), 0);
    }

    /// A badge that never answers can't hang the CLI.
    #[test]
    fn a_silent_server_times_out() {
        let endpoint = endpoint("hung");
        // Bound, but nobody serves it: a hung badge.
        let _listener = NamedPipe::bind(&endpoint).unwrap();
        let start = Instant::now();
        match ipc::call(&endpoint, Request::Status) {
            Err(CallError::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::TimedOut, "{e}"),
            other => panic!("want a timeout, got {other:?}"),
        }
        let took = start.elapsed();
        assert!(
            took >= TIMEOUT - Duration::from_millis(100) && took < TIMEOUT * 2,
            "{took:?}"
        );
    }

    /// A pipe another owner created (here Administrators, when this token
    /// may assign it) is refused before any request is sent.
    #[test]
    fn a_pipe_owned_by_someone_else_is_refused() {
        let endpoint = endpoint("owner");
        let name = wide(&endpoint);
        let sddl: Vec<u16> = "O:BAD:(A;;GA;;;WD)".encode_utf16().chain([0]).collect();
        let mut sd = null_mut();
        // SAFETY: as in `instance`; the descriptor outlives the call.
        let handle = unsafe {
            assert_ne!(
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    SDDL_REVISION_1,
                    &mut sd,
                    null_mut()
                ),
                0
            );
            let _sd = Descriptor(sd);
            let attributes = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd,
                bInheritHandle: 0,
            };
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE,
                1,
                4096,
                4096,
                0,
                &attributes,
            )
        };
        let Ok(_squatter) = owned(handle) else {
            println!(
                "SKIP: this token can't make Administrators an owner: {}",
                io::Error::last_os_error()
            );
            return;
        };
        match ipc::call(&endpoint, Request::Status) {
            Err(CallError::Io(e)) => {
                assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{e}");
                assert!(e.to_string().contains("another user"), "{e}");
            }
            other => panic!("want a refusal, got {other:?}"),
        }
    }
}
