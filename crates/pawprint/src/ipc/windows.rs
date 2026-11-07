//! Named-pipe transport for Windows (SPEC §10.4): `\\.\pipe\pawprint-<user>-<sid>`.
//!
//! Hook for the Windows backend. Replace these bodies with a named-pipe
//! server (`CreateNamedPipeW` with `FILE_FLAG_FIRST_PIPE_INSTANCE`, mapping
//! "already exists" to `ErrorKind::AddrInUse`) and client (`CreateFileW`,
//! mapping "no such pipe" to `ErrorKind::NotFound`). Pipes vanish with their
//! server, so `remove_stale` has nothing to do.

use std::io;
use std::path::Path;

use super::{Stream, Transport};

pub struct NamedPipe;

/// The server end of the pipe (to be filled in with the pipe handle).
pub struct PipeListener;

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "the Windows named-pipe transport is not implemented yet (M1)",
    )
}

impl Transport for NamedPipe {
    type Listener = PipeListener;

    fn bind(_endpoint: &Path) -> io::Result<PipeListener> {
        Err(unsupported())
    }

    fn accept(_listener: &PipeListener) -> io::Result<Box<dyn Stream>> {
        Err(unsupported())
    }

    fn connect(_endpoint: &Path) -> io::Result<Box<dyn Stream>> {
        Err(unsupported())
    }

    fn remove_stale(_endpoint: &Path) -> io::Result<()> {
        Ok(())
    }
}
