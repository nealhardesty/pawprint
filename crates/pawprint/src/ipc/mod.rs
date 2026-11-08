//! The IPC protocol, server loop, and client (SPEC §10.5).
//!
//! Newline-delimited JSON, one request and one response per connection. The
//! byte transport is the only platform-specific part: Unix sockets on Linux
//! and macOS ([`unix`]), named pipes on Windows ([`windows`]), both behind
//! [`Transport`].

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub mod windows;

/// Protocol version. The CLI and the badge are the same binary, so there are
/// no compatibility shims beyond this check.
pub const VERSION: u32 = 1;

/// How long either side waits for the other.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// A connected byte stream to or from the badge.
pub trait Stream: Read + Write + Send {}
impl<T: Read + Write + Send> Stream for T {}

/// One platform's IPC transport. `bind` doubles as the single-instance lock
/// (SPEC §10.4), so it must fail with [`io::ErrorKind::AddrInUse`] while the
/// endpoint exists.
pub trait Transport {
    type Listener: Send + 'static;
    fn bind(endpoint: &Path) -> io::Result<Self::Listener>;
    /// Wait for the next client. The stream should time out after [`TIMEOUT`].
    fn accept(listener: &Self::Listener) -> io::Result<Box<dyn Stream>>;
    /// Connect to a running badge, failing fast if there is none.
    fn connect(endpoint: &Path) -> io::Result<Box<dyn Stream>>;
    /// Remove an endpoint that nobody answers on (a crashed badge).
    fn remove_stale(endpoint: &Path) -> io::Result<()>;
}

#[cfg(unix)]
pub type Platform = unix::UnixSocket;
#[cfg(windows)]
pub type Platform = windows::NamedPipe;

pub type Listener = <Platform as Transport>::Listener;

/// The commands a client can send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    Status,
    Move,
    ResetPosition,
    Reload,
    Quit,
}

impl Request {
    pub const ALL: [Request; 5] = [
        Request::Status,
        Request::Move,
        Request::ResetPosition,
        Request::Reload,
        Request::Quit,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Request::Status => "status",
            Request::Move => "move",
            Request::ResetPosition => "reset-position",
            Request::Reload => "reload",
            Request::Quit => "quit",
        }
    }

    pub fn parse(s: &str) -> Option<Request> {
        Request::ALL.into_iter().find(|r| r.name() == s)
    }
}

/// What `pawprint status` prints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub pid: u32,
    pub backend: String,
    pub session: String,
    /// Target display name, if there is one.
    pub display: Option<String>,
    /// Badge rectangle in physical desktop pixels: x, y, width, height.
    pub rect: Option<[i64; 4]>,
    pub scale: Option<f64>,
    pub mascot: String,
    pub label: Option<String>,
    pub position: String,
    pub position_source: String,
    pub size: u32,
    pub moving: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub v: u32,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
}

impl Response {
    pub fn ok() -> Response {
        Response {
            v: VERSION,
            ok: true,
            error: None,
            status: None,
        }
    }

    pub fn error(msg: impl Into<String>) -> Response {
        Response {
            ok: false,
            error: Some(msg.into()),
            ..Response::ok()
        }
    }

    pub fn status(status: Status) -> Response {
        Response {
            status: Some(status),
            ..Response::ok()
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Wire {
    v: u32,
    cmd: String,
}

/// A request waiting for the badge's event loop to answer it.
pub struct Incoming {
    pub request: Request,
    reply: mpsc::Sender<Response>,
}

impl Incoming {
    pub fn reply(self, response: Response) {
        let _ = self.reply.send(response);
    }
}

/// Parse one request line, or say what is wrong with it.
fn parse_request(line: &str) -> Result<Request, String> {
    let wire: Wire = serde_json::from_str(line.trim()).map_err(|e| format!("bad request: {e}"))?;
    if wire.v != VERSION {
        return Err(format!(
            "unsupported protocol version {} (this badge speaks {VERSION})",
            wire.v
        ));
    }
    Request::parse(&wire.cmd).ok_or_else(|| format!("unknown command {:?}", wire.cmd))
}

fn write_line(stream: &mut dyn Stream, value: &impl Serialize) -> io::Result<()> {
    let mut line = serde_json::to_vec(value).map_err(io::Error::other)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    stream.flush()
}

/// Read one line of at most 4 KiB.
fn read_line(stream: &mut dyn Stream) -> io::Result<String> {
    let mut line = String::new();
    BufReader::new(stream.take(4096)).read_line(&mut line)?;
    Ok(line)
}

/// Serve requests on a background thread. Each valid request is handed to
/// `post`, which forwards it into the backend's native event loop (a calloop
/// channel on Linux, a posted message on Windows, the main queue on macOS);
/// the loop answers through [`Incoming::reply`]. The thread ends when `post`
/// returns false, meaning the loop is gone.
pub fn serve(listener: Listener, post: impl Fn(Incoming) -> bool + Send + 'static) {
    std::thread::spawn(move || {
        loop {
            let mut stream = match Platform::accept(&listener) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("ipc: accept failed: {e}");
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
            };
            let response = match read_line(&mut *stream).map(|l| parse_request(&l)) {
                Err(e) => {
                    log::debug!("ipc: read failed: {e}");
                    continue;
                }
                Ok(Err(e)) => Response::error(e),
                Ok(Ok(request)) => {
                    let (tx, rx) = mpsc::channel();
                    if !post(Incoming { request, reply: tx }) {
                        let _ = write_line(
                            &mut *stream,
                            &Response::error("the badge is shutting down"),
                        );
                        return;
                    }
                    rx.recv_timeout(TIMEOUT)
                        .unwrap_or_else(|_| Response::error("the badge did not answer in time"))
                }
            };
            if let Err(e) = write_line(&mut *stream, &response) {
                log::debug!("ipc: write failed: {e}");
            }
        }
    });
}

/// Why a call to the badge failed.
#[derive(Debug)]
pub enum CallError {
    /// Nothing is listening: no badge runs in this session.
    NotRunning,
    Io(io::Error),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::NotRunning => f.write_str("PAWPRINT is not running in this session"),
            CallError::Io(e) => write!(f, "cannot talk to the running badge: {e}"),
        }
    }
}

/// Send one request to the badge at `endpoint` and wait for its answer.
pub fn call(endpoint: &Path, request: Request) -> Result<Response, CallError> {
    let mut stream = Platform::connect(endpoint).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => CallError::NotRunning,
        _ => CallError::Io(e),
    })?;
    let io = |e| CallError::Io(e);
    write_line(
        &mut *stream,
        &Wire {
            v: VERSION,
            cmd: request.name().into(),
        },
    )
    .map_err(io)?;
    let line = read_line(&mut *stream).map_err(io)?;
    serde_json::from_str(line.trim()).map_err(|e| CallError::Io(io::Error::other(e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_names_round_trip() {
        for r in Request::ALL {
            assert_eq!(Request::parse(r.name()), Some(r));
        }
        assert_eq!(Request::parse("dance"), None);
    }

    #[test]
    fn parses_requests_and_rejects_bad_ones() {
        assert_eq!(
            parse_request("{\"v\":1,\"cmd\":\"move\"}\n"),
            Ok(Request::Move)
        );
        let bad_version = parse_request("{\"v\":2,\"cmd\":\"move\"}").unwrap_err();
        assert!(bad_version.contains("version 2"));
        assert!(parse_request("{\"v\":1,\"cmd\":\"dance\"}").is_err());
        assert!(parse_request("not json").is_err());
    }

    #[test]
    fn responses_match_the_spec() {
        assert_eq!(
            serde_json::to_string(&Response::ok()).unwrap(),
            "{\"v\":1,\"ok\":true}"
        );
        assert_eq!(
            serde_json::to_string(&Response::error("no target display")).unwrap(),
            "{\"v\":1,\"ok\":false,\"error\":\"no target display\"}"
        );
    }
}
