//! xlit-ipc — the wire between frontends and `xlit-daemon`.
//!
//! The engine's data (dictionary FST, learning store) is worth loading once per
//! machine, not once per application: a Windows TSF text service is loaded into
//! *every* process that takes input. So the daemon owns the [`xlit_core::Engine`]
//! and frontends become thin clients speaking this protocol.
//!
//! Transport is a Unix domain socket on Linux/macOS and a named pipe on
//! Windows — see [`transport`]. Both are local-only and permissioned to the
//! current user; nothing here is expected to cross a machine boundary, which is
//! why the framing is as small as it is:
//!
//! ```text
//! [u32 little-endian byte length][JSON body]
//! ```
//!
//! One request, one response, synchronously, per connection — but a connection
//! may carry any number of those pairs, so a frontend connects once and keeps
//! the handle for the life of the session.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

pub mod transport;

pub use transport::{connect, default_endpoint, Listener, Stream};

/// Refuse absurd frames rather than allocating whatever a peer claims. A
/// composition buffer is a handful of bytes; a megabyte is already generous.
pub const MAX_FRAME: usize = 1024 * 1024;

/// A request from a frontend. Serialised with the operation in an `"op"` field:
/// `{"op":"candidates","text":"namaste"}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Request {
    /// Ranked candidates for a Latin buffer.
    Candidates {
        text: String,
        /// Most candidates to return. `None` means the daemon's own cap.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// Record a deliberate pick, so the learning store can boost it next time.
    Commit { input: String, chosen: String },
    /// Straight rule-engine output, no ranking layers.
    Transliterate { text: String },
    /// Liveness check — also how a client tells a stale socket from a live one.
    Ping,
    /// Ask the daemon to exit once in-flight connections finish.
    Shutdown,
}

/// One ranked candidate, flattened for the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub text: String,
    /// [`xlit_core::Source`] as a lowercase string (`"rule"`, `"dictionary"`,
    /// `"confirmed"`, `"learned"`, `"model"`, `"raw"`). A string rather than an
    /// enum so adding a source later cannot break an older client.
    pub source: String,
    pub score: i32,
}

/// The answer to a [`Request`]. Fields are omitted when they do not apply, so
/// a commit really does come back as `{"ok":true}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<Candidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    pub fn ok() -> Self {
        Response { ok: true, ..Default::default() }
    }

    pub fn candidates(candidates: Vec<Candidate>) -> Self {
        Response { ok: true, candidates, ..Default::default() }
    }

    pub fn text(text: String) -> Self {
        Response { ok: true, text: Some(text), ..Default::default() }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        Response { ok: false, error: Some(msg.into()), ..Default::default() }
    }
}

/// Write one length-prefixed JSON frame.
pub fn write_frame<W: Write, T: Serialize>(w: &mut W, value: &T) -> io::Result<()> {
    let body = serde_json::to_vec(value).map_err(io::Error::other)?;
    if body.len() > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "frame too large"));
    }
    // One write, not two: on a named pipe in message mode each write is its own
    // message, and a split header would arrive as a message of its own.
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
    buf.extend_from_slice(&body);
    w.write_all(&buf)?;
    w.flush()
}

/// Read one length-prefixed JSON frame. `Ok(None)` means the peer hung up
/// cleanly between frames, which is the normal way a session ends.
pub fn read_frame<R: Read, T: for<'de> Deserialize<'de>>(r: &mut R) -> io::Result<Option<T>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        // A named-pipe peer that closes mid-poll surfaces as BrokenPipe here.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(io::Error::other)
}

/// A frontend's handle on the daemon.
///
/// Deliberately blocking and single-threaded: a keystroke's worth of work over
/// a local socket is microseconds, and an input method that answered
/// asynchronously would have to hold composition state to match up replies.
pub struct Client {
    stream: Stream,
}

impl Client {
    /// Connect to the daemon at the default endpoint for this platform.
    pub fn connect() -> io::Result<Self> {
        Self::connect_at(&default_endpoint())
    }

    pub fn connect_at(endpoint: &str) -> io::Result<Self> {
        Ok(Client { stream: connect(endpoint)? })
    }

    /// Send one request, wait for its response.
    pub fn call(&mut self, req: &Request) -> io::Result<Response> {
        write_frame(&mut self.stream, req)?;
        read_frame(&mut self.stream)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "daemon closed connection"))
    }

    pub fn candidates(&mut self, text: &str, limit: Option<usize>) -> io::Result<Vec<Candidate>> {
        let r = self.call(&Request::Candidates { text: text.to_string(), limit })?;
        response_or_err(r).map(|r| r.candidates)
    }

    pub fn commit(&mut self, input: &str, chosen: &str) -> io::Result<()> {
        let r = self.call(&Request::Commit {
            input: input.to_string(),
            chosen: chosen.to_string(),
        })?;
        response_or_err(r).map(|_| ())
    }

    pub fn transliterate(&mut self, text: &str) -> io::Result<String> {
        let r = self.call(&Request::Transliterate { text: text.to_string() })?;
        Ok(response_or_err(r)?.text.unwrap_or_default())
    }

    pub fn ping(&mut self) -> io::Result<()> {
        let r = self.call(&Request::Ping)?;
        response_or_err(r).map(|_| ())
    }
}

fn response_or_err(r: Response) -> io::Result<Response> {
    if r.ok {
        Ok(r)
    } else {
        Err(io::Error::other(
            r.error.unwrap_or_else(|| "daemon reported failure".to_string()),
        ))
    }
}

/// Serve requests on `listener` until a client asks to shut down.
///
/// `handle` runs on a worker thread per connection, so it must be shareable;
/// the daemon's own state is behind `&self`-style interior mutability already
/// (the learning store takes its own lock).
pub fn serve<F>(listener: Listener, handle: F) -> io::Result<()>
where
    F: Fn(Request) -> Response + Send + Sync + 'static,
{
    let handle = std::sync::Arc::new(handle);
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));

    while running.load(std::sync::atomic::Ordering::SeqCst) {
        let stream = match listener.accept() {
            Ok(s) => s,
            // A single bad connection must never take the daemon down with it.
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                eprintln!("xlit-daemon: accept failed: {e}");
                continue;
            }
        };
        let handle = handle.clone();
        let running = running.clone();
        let endpoint = listener.endpoint().to_string();
        std::thread::spawn(move || {
            if let Err(e) = serve_connection(stream, &*handle, &running, &endpoint) {
                eprintln!("xlit-daemon: connection ended: {e}");
            }
        });
    }
    Ok(())
}

fn serve_connection<F>(
    mut stream: Stream,
    handle: &F,
    running: &std::sync::atomic::AtomicBool,
    endpoint: &str,
) -> io::Result<()>
where
    F: Fn(Request) -> Response,
{
    loop {
        let req: Request = match read_frame(&mut stream) {
            Ok(Some(r)) => r,
            Ok(None) => return Ok(()),
            // Malformed JSON is the client's bug, not a reason to drop the
            // session: say so and keep reading.
            Err(e) if e.kind() == io::ErrorKind::Other => {
                write_frame(&mut stream, &Response::error(e.to_string()))?;
                continue;
            }
            Err(e) => return Err(e),
        };
        let shutdown = matches!(req, Request::Shutdown);
        let resp = handle(req);
        write_frame(&mut stream, &resp)?;
        if shutdown {
            running.store(false, std::sync::atomic::Ordering::SeqCst);
            // The accept loop is parked inside `accept()` and will not look at
            // the flag until something arrives: connect to ourselves so it does.
            let _ = connect(endpoint);
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, &Request::Ping).unwrap();
        write_frame(
            &mut buf,
            &Request::Candidates { text: "namaste".into(), limit: Some(5) },
        )
        .unwrap();

        let mut cursor = io::Cursor::new(buf);
        assert!(matches!(
            read_frame::<_, Request>(&mut cursor).unwrap(),
            Some(Request::Ping)
        ));
        match read_frame::<_, Request>(&mut cursor).unwrap() {
            Some(Request::Candidates { text, limit }) => {
                assert_eq!(text, "namaste");
                assert_eq!(limit, Some(5));
            }
            other => panic!("unexpected frame: {other:?}"),
        }
        assert!(read_frame::<_, Request>(&mut cursor).unwrap().is_none());
    }

    #[test]
    fn commit_response_is_just_ok() {
        let json = serde_json::to_string(&Response::ok()).unwrap();
        assert_eq!(json, r#"{"ok":true}"#);
    }

    #[test]
    fn request_wire_shape_matches_the_documented_protocol() {
        let json = serde_json::to_string(&Request::Candidates {
            text: "namaste".into(),
            limit: None,
        })
        .unwrap();
        assert_eq!(json, r#"{"op":"candidates","text":"namaste"}"#);

        let parsed: Request =
            serde_json::from_str(r#"{"op":"commit","input":"namaste","chosen":"नमस्ते"}"#).unwrap();
        match parsed {
            Request::Commit { input, chosen } => {
                assert_eq!(input, "namaste");
                assert_eq!(chosen, "नमस्ते");
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }

    #[test]
    fn oversized_frame_is_refused_not_allocated() {
        let mut bytes = (MAX_FRAME as u32 + 1).to_le_bytes().to_vec();
        bytes.extend_from_slice(b"{}");
        let err = read_frame::<_, Request>(&mut io::Cursor::new(bytes)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
