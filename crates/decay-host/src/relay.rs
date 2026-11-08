//! The relay (Software Design §4.8): browser stdio ⇄ app IPC.
//!
//! 1. The caller's origin (the browser's first argument) must be an allowed extension.
//! 2. The first message must be `hello`; it says which browser and profile this is.
//! 3. Connect to the app. Only when `hello.launch` is set (browser start-up, or the user
//!    asked) is the app started and waited for (up to 3 s): a reconnect must not reopen
//!    an app the user quit. If the app can't be reached the extension gets
//!    `error APP_UNAVAILABLE` and stays in standalone mode.
//! 4. Then every extension message goes to the app as a [`HostFrame`] (forwarded as
//!    received, not rebuilt), and every app message comes back, split into chunks when
//!    over the browser's 1 MB limit.
//!
//! The host never interprets messages beyond that: the app owns the protocol. When the
//! app goes away, everything it already sent is delivered before the extension is told.

use std::io::{self, Read, Write};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use decay_bridge::chunk::split;
use decay_bridge::frame::{
    MAX_FROM_BROWSER, MAX_IPC_FRAME, MAX_TO_BROWSER, read_frame, write_envelope,
};
use decay_bridge::ipc::{ConnInfo, Endpoint, IpcReader, IpcWriter, connect, host_frame_json};
use decay_bridge::protocol::{Envelope, ErrorCode, Hello, MAX_PROFILE_ID_LEN, kinds};
use decay_bridge::text::{decode_envelope, sanitize_surrogates};
use serde_json::value::RawValue;

use crate::config::origin_allowed;

/// How long to wait, after a write to the app failed, for its last messages to arrive.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(3);

/// How the relay ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The caller isn't an allowed extension: nothing was read or written.
    RejectedOrigin,
    /// The first message wasn't a valid `hello`.
    BadHello,
    /// The app couldn't be reached; the extension was told.
    AppUnavailable,
    /// The browser closed the connection.
    BrowserClosed,
    /// The app closed the connection; the extension was told.
    AppClosed,
}

/// The end of a run. Hold [`Finished::output`]'s lock while exiting the process, so no
/// relay thread is cut off in the middle of a frame.
#[derive(Debug)]
pub struct Finished<W> {
    pub outcome: Outcome,
    pub output: Arc<Mutex<W>>,
}

/// What the relay needs.
pub struct Relay {
    /// The caller, from the browser's first argument.
    pub origin: String,
    pub allowed: Vec<String>,
    pub endpoint: io::Result<Endpoint>,
    /// Starts the app; called at most once, and only when `hello.launch` is set.
    pub launch_app: Box<dyn FnOnce() + Send>,
    /// How long to keep trying after starting the app.
    pub connect_timeout: Duration,
    /// Reported to the app as `conn.pid`.
    pub pid: u32,
}

impl std::fmt::Debug for Relay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Relay")
            .field("origin", &self.origin)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

type SharedOut<W> = Arc<Mutex<W>>;

enum SendError {
    /// The browser side is gone.
    Io,
    /// This message can't be delivered (too big even in chunks); others still can.
    Undeliverable,
}

/// Writes a message (chunked if needed) while holding the output lock, so frames from the
/// two relay threads never interleave and a chunk group is written as a whole.
fn send<W: Write>(out: &SharedOut<W>, env: &Envelope) -> Result<(), SendError> {
    let frames = split(env, MAX_TO_BROWSER).map_err(|_| SendError::Undeliverable)?;
    let mut w = out.lock().map_err(|_| SendError::Io)?;
    for frame in &frames {
        write_envelope(&mut *w, frame).map_err(|_| SendError::Io)?;
    }
    Ok(())
}

fn connect_retrying(
    endpoint: &io::Result<Endpoint>,
    timeout: Duration,
) -> Option<(IpcReader, IpcWriter)> {
    let deadline = Instant::now() + timeout;
    loop {
        // On Windows the endpoint is read again: the app publishes its pipe once started.
        let ep = match endpoint {
            Ok(ep) => Some(ep.clone()),
            Err(_) => Endpoint::default_for_user().ok(),
        };
        if let Some(conn) = ep.and_then(|ep| connect(&ep).ok()) {
            return Some(conn);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// A valid `hello`, or `None`.
fn parse_hello(first: Option<&[u8]>) -> Option<(Envelope, Hello)> {
    let env = decode_envelope(first?).ok()?;
    if env.kind != kinds::HELLO || env.chunk.is_some() {
        return None;
    }
    let hello: Hello = env.payload_as().ok()?;
    let id_ok = !hello.profile_id.is_empty() && hello.profile_id.len() <= MAX_PROFILE_ID_LEN;
    id_ok.then_some((env, hello))
}

/// An extension message as JSON text to forward: valid UTF-8 (invalid bytes and lone
/// surrogates become U+FFFD) and a JSON object. Not parsed into a tree.
fn checked_message(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let text = sanitize_surrogates(&text).into_owned();
    let raw: &RawValue = serde_json::from_str(&text).ok()?;
    raw.get().starts_with('{').then_some(text)
}

enum Ended {
    Browser,
    App,
    /// Writing to the app failed: wait briefly for the reader to deliver what's left.
    AppWriteFailed,
}

impl Relay {
    /// Runs until the browser or the app closes the connection. Relay threads may still be
    /// blocked on the other side when this returns: the caller exits the process (as
    /// `decay-host` does), holding [`Finished::output`]'s lock.
    pub fn run<R, W>(self, mut stdin: R, stdout: W) -> Finished<W>
    where
        R: Read + Send + 'static,
        W: Write + Send + 'static,
    {
        let out = Arc::new(Mutex::new(stdout));
        let finish = |outcome| Finished { outcome, output: Arc::clone(&out) };
        if !origin_allowed(&self.origin, &self.allowed) {
            return finish(Outcome::RejectedOrigin);
        }

        let first = read_frame(&mut stdin, MAX_FROM_BROWSER).ok().flatten();
        let Some((hello_env, hello)) = parse_hello(first.as_deref()) else {
            let _ = send(&out, &Envelope::error(None, ErrorCode::BadMessage, "expected hello"));
            return finish(Outcome::BadHello);
        };

        let conn = ConnInfo {
            browser: hello.browser,
            profile_id: hello.profile_id,
            pid: self.pid,
            origin: self.origin.clone(),
        };
        let mut app = connect_retrying(&self.endpoint, Duration::ZERO);
        if app.is_none() && hello.launch {
            (self.launch_app)();
            app = connect_retrying(&self.endpoint, self.connect_timeout);
        }
        let unavailable =
            |why: &str| Envelope::error(Some(&hello_env), ErrorCode::AppUnavailable, why);
        let Some((mut app_rx, mut app_tx)) = app else {
            let _ = send(&out, &unavailable("Decay isn't running"));
            return finish(Outcome::AppUnavailable);
        };
        let hello_frame = first
            .as_deref()
            .and_then(checked_message)
            .and_then(|msg| host_frame_json(&conn, &msg).ok());
        if hello_frame.and_then(|f| app_tx.write_raw(&f).ok()).is_none() {
            let _ = send(&out, &unavailable("Decay closed"));
            return finish(Outcome::AppUnavailable);
        }

        let (done_tx, done_rx) = mpsc::channel();
        let app_done = done_tx.clone();
        let app_out = Arc::clone(&out);
        thread::spawn(move || {
            // App → browser. A frame that isn't a valid message is dropped; a broken
            // stream or the app closing ends the session.
            while let Ok(Some(bytes)) = app_rx.read_raw() {
                let Ok(env) = decode_envelope(&bytes) else { continue };
                match send(&app_out, &env) {
                    Ok(()) | Err(SendError::Undeliverable) => {}
                    Err(SendError::Io) => {
                        let _ = app_done.send(Ended::Browser);
                        return;
                    }
                }
            }
            let _ =
                send(&app_out, &Envelope::error(None, ErrorCode::AppUnavailable, "Decay closed"));
            let _ = app_done.send(Ended::App);
        });
        let browser_out = Arc::clone(&out);
        thread::spawn(move || {
            // Browser → app. Ends at end of input, or on a stream that can't be trusted.
            while let Ok(Some(bytes)) = read_frame(&mut stdin, MAX_FROM_BROWSER) {
                let frame = checked_message(&bytes)
                    .and_then(|msg| host_frame_json(&conn, &msg).ok())
                    .filter(|f| f.len() <= MAX_IPC_FRAME);
                let Some(frame) = frame else {
                    let err = Envelope::error(
                        None,
                        ErrorCode::BadMessage,
                        "invalid or oversized message",
                    );
                    if send(&browser_out, &err).is_err() {
                        break;
                    }
                    continue;
                };
                if app_tx.write_raw(&frame).is_err() {
                    let _ = done_tx.send(Ended::AppWriteFailed);
                    return;
                }
            }
            let _ = done_tx.send(Ended::Browser);
        });
        let outcome = match done_rx.recv() {
            Ok(Ended::App) => Outcome::AppClosed,
            Ok(Ended::AppWriteFailed) => {
                // The app went away while the browser was talking: let the reader deliver
                // what the app sent before it closed, and tell the extension.
                match done_rx.recv_timeout(DRAIN_TIMEOUT) {
                    Ok(Ended::Browser) => Outcome::BrowserClosed,
                    Ok(_) => Outcome::AppClosed,
                    Err(_) => {
                        let _ = send(
                            &out,
                            &Envelope::error(None, ErrorCode::AppUnavailable, "Decay closed"),
                        );
                        Outcome::AppClosed
                    }
                }
            }
            _ => Outcome::BrowserClosed,
        };
        finish(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::origin_of;
    use decay_bridge::frame::{read_envelope, write_frame};
    use decay_bridge::ipc::IpcServer;
    use decay_bridge::protocol::{BrowserKind, ErrorPayload};
    use serde_json::json;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicBool, Ordering};

    const ID: &str = "abcdefghijklmnopabcdefghijklmnop";

    /// A `Write` the test can read afterwards.
    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Sink {
        fn envelopes(&self) -> Vec<Envelope> {
            let bytes = self.0.lock().unwrap().clone();
            let mut r = Cursor::new(bytes);
            std::iter::from_fn(|| read_envelope(&mut r, usize::MAX).unwrap()).collect()
        }
    }

    fn endpoint(dir: &std::path::Path) -> Endpoint {
        if cfg!(windows) {
            Endpoint::Pipe(format!("decay-test-{}", uuid_like()))
        } else {
            Endpoint::Socket(dir.join("run").join("decay.sock"))
        }
    }

    fn uuid_like() -> String {
        format!(
            "{:x}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        )
    }

    fn relay(endpoint: Endpoint, launch: Box<dyn FnOnce() + Send>) -> Relay {
        Relay {
            origin: origin_of(ID),
            allowed: vec![origin_of(ID)],
            endpoint: Ok(endpoint),
            launch_app: launch,
            connect_timeout: Duration::from_millis(500),
            pid: 7,
        }
    }

    fn hello_with(launch: bool, profile_id: &str) -> Envelope {
        Envelope::new(
            kinds::HELLO,
            &json!({"extVersion": "1.0.0", "browser": "edge", "profileId": profile_id,
                    "capabilities": [], "launch": launch}),
        )
        .unwrap()
    }

    fn hello() -> Envelope {
        hello_with(false, "prof")
    }

    fn stdin(messages: &[Vec<u8>]) -> Cursor<Vec<u8>> {
        let mut buf = Vec::new();
        for m in messages {
            write_frame(&mut buf, m).unwrap();
        }
        Cursor::new(buf)
    }

    fn json_bytes(env: &Envelope) -> Vec<u8> {
        serde_json::to_vec(env).unwrap()
    }

    #[test]
    fn rejects_unknown_callers_without_a_word() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = relay(endpoint(dir.path()), Box::new(|| {}));
        r.origin = origin_of(&ID.replace('a', "c"));
        let out = Sink::default();
        assert_eq!(
            r.run(stdin(&[json_bytes(&hello())]), out.clone()).outcome,
            Outcome::RejectedOrigin
        );
        assert_eq!(out.envelopes().len(), 0);
    }

    #[test]
    fn needs_a_hello_first() {
        let dir = tempfile::tempdir().unwrap();
        let out = Sink::default();
        let first = Envelope::new(kinds::HEARTBEAT, &json!({"at": 1})).unwrap();
        let outcome = relay(endpoint(dir.path()), Box::new(|| {}))
            .run(stdin(&[json_bytes(&first)]), out.clone())
            .outcome;
        assert_eq!(outcome, Outcome::BadHello);
        let reply = out.envelopes().pop().unwrap();
        assert_eq!(reply.payload_as::<ErrorPayload>().unwrap().code, ErrorCode::BadMessage);
    }

    #[test]
    fn tells_the_extension_when_the_app_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let launched = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&launched);
        let out = Sink::default();
        let h = hello_with(true, "prof");
        let outcome =
            relay(endpoint(dir.path()), Box::new(move || flag.store(true, Ordering::SeqCst)))
                .run(stdin(&[json_bytes(&h)]), out.clone())
                .outcome;
        assert_eq!(outcome, Outcome::AppUnavailable);
        assert!(launched.load(Ordering::SeqCst), "tried to start the app");
        let reply = out.envelopes().pop().unwrap();
        assert_eq!(reply.reply_to.as_deref(), Some(h.id.as_str()));
        assert_eq!(reply.payload_as::<ErrorPayload>().unwrap().code, ErrorCode::AppUnavailable);
    }

    #[test]
    fn a_reconnect_never_starts_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let out = Sink::default();
        let start = Instant::now();
        let outcome = relay(endpoint(dir.path()), Box::new(|| panic!("started the app")))
            .run(stdin(&[json_bytes(&hello())]), out.clone())
            .outcome;
        assert_eq!(outcome, Outcome::AppUnavailable);
        assert!(start.elapsed() < Duration::from_millis(400), "no waiting either");
    }

    #[test]
    fn refuses_an_oversized_profile_id() {
        let dir = tempfile::tempdir().unwrap();
        let out = Sink::default();
        let h = hello_with(false, &"p".repeat(decay_bridge::protocol::MAX_PROFILE_ID_LEN + 1));
        let outcome = relay(endpoint(dir.path()), Box::new(|| {}))
            .run(stdin(&[json_bytes(&h)]), out.clone())
            .outcome;
        assert_eq!(outcome, Outcome::BadHello);
    }

    #[test]
    fn skips_bad_app_frames_and_delivers_everything_before_the_app_quits() {
        let dir = tempfile::tempdir().unwrap();
        let ep = endpoint(dir.path());
        let server = IpcServer::bind(ep.clone()).unwrap();
        let app = thread::spawn(move || {
            let (mut rx, mut tx) = server.accept().unwrap();
            let first = rx.read_host_frame().unwrap().unwrap();
            tx.write_raw(b"[1]").unwrap(); // not a message: skipped
            tx.write_raw(b"").unwrap();
            let big =
                first.msg.reply(kinds::WELCOME, &json!({"big": "w".repeat(3_000_000)})).unwrap();
            tx.write_json(&big).unwrap();
            big
            // The app quits here.
        });
        // The browser keeps talking while the app goes away.
        let (feed, pipe_in) = blocking_stdin(&json_bytes(&hello()));
        let beat = Envelope::new(kinds::HEARTBEAT, &json!({"at": 1})).unwrap();
        let mut framed = Vec::new();
        write_frame(&mut framed, &json_bytes(&beat)).unwrap();
        feed.send(framed).unwrap();
        let out = Sink::default();
        let finished = relay(ep, Box::new(|| {})).run(pipe_in, out.clone());
        let welcome = app.join().unwrap();
        assert_eq!(finished.outcome, Outcome::AppClosed);
        let _held = finished.output.lock().unwrap();
        let got = out.envelopes();
        let mut reassembler = decay_bridge::chunk::Reassembler::new();
        let joined: Vec<Envelope> =
            got.iter().filter_map(|e| reassembler.push(e.clone(), 0).unwrap()).collect();
        assert_eq!(joined.first(), Some(&welcome), "the whole reply arrived");
        let last = got.last().unwrap();
        assert_eq!(last.payload_as::<ErrorPayload>().unwrap().code, ErrorCode::AppUnavailable);
    }

    #[test]
    fn relays_both_ways_and_chunks_big_replies() {
        let dir = tempfile::tempdir().unwrap();
        let ep = endpoint(dir.path());
        let server = IpcServer::bind(ep.clone()).unwrap();
        let app = thread::spawn(move || {
            let (mut rx, mut tx) = server.accept().unwrap();
            let first = rx.read_host_frame().unwrap().unwrap();
            assert_eq!(first.conn.browser, BrowserKind::Edge);
            assert_eq!((first.conn.profile_id.as_str(), first.conn.pid), ("prof", 7));
            assert_eq!(first.conn.origin, origin_of(ID));
            let welcome =
                first.msg.reply(kinds::WELCOME, &json!({"big": "w".repeat(2_500_000)})).unwrap();
            tx.write_json(&welcome).unwrap();
            // The heartbeat follows (the invalid message in between is never forwarded).
            let next = rx.read_host_frame().unwrap().unwrap();
            (vec![first.msg.kind, next.msg.kind], welcome)
        });
        let out = Sink::default();
        let beat = Envelope::new(kinds::HEARTBEAT, &json!({"at": 1})).unwrap();
        let input = stdin(&[json_bytes(&hello()), b"not json".to_vec(), json_bytes(&beat)]);
        let outcome =
            relay(ep, Box::new(|| panic!("the app was running"))).run(input, out.clone()).outcome;
        assert_eq!(outcome, Outcome::BrowserClosed);
        let (got, welcome) = app.join().unwrap();
        assert_eq!(got, ["hello", "heartbeat"], "the invalid message wasn't forwarded");

        // Everything the extension received fits the browser's limit, and the chunks join
        // back into the welcome. (The relay thread may still be writing when `run`
        // returns: wait for the reply.)
        let deadline = Instant::now() + Duration::from_secs(10);
        while out.envelopes().iter().filter(|e| e.chunk.is_some()).count() < 3
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(20));
        }
        let mut reassembler = decay_bridge::chunk::Reassembler::new();
        let mut joined = None;
        let mut bad_message = false;
        for env in out.envelopes() {
            assert!(serde_json::to_vec(&env).unwrap().len() <= MAX_TO_BROWSER);
            if env.kind == kinds::ERROR {
                bad_message = true;
            } else if let Some(full) = reassembler.push(env, 0).unwrap() {
                joined = Some(full);
            }
        }
        assert!(bad_message, "the extension heard about its invalid message");
        assert!(joined == Some(welcome), "the welcome arrived whole");
    }

    #[test]
    fn tells_the_extension_when_the_app_quits() {
        let dir = tempfile::tempdir().unwrap();
        let ep = endpoint(dir.path());
        let server = IpcServer::bind(ep.clone()).unwrap();
        let app = thread::spawn(move || {
            let (mut rx, _tx) = server.accept().unwrap();
            rx.read_host_frame().unwrap().unwrap();
            // The app quits: both halves dropped.
        });
        // stdin that never ends (the browser keeps the port open).
        let (_keep, pipe_in) = blocking_stdin(&json_bytes(&hello()));
        let out = Sink::default();
        let outcome = relay(ep, Box::new(|| {})).run(pipe_in, out.clone()).outcome;
        app.join().unwrap();
        assert_eq!(outcome, Outcome::AppClosed);
        let last = out.envelopes().pop().unwrap();
        assert_eq!(last.payload_as::<ErrorPayload>().unwrap().code, ErrorCode::AppUnavailable);
    }

    /// A reader that yields `first` as a frame and then blocks until the sender is dropped.
    fn blocking_stdin(first: &[u8]) -> (mpsc::Sender<Vec<u8>>, impl Read + Send + 'static) {
        struct ChanReader {
            rx: mpsc::Receiver<Vec<u8>>,
            buf: Cursor<Vec<u8>>,
        }
        impl Read for ChanReader {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                loop {
                    let n = self.buf.read(out)?;
                    if n > 0 {
                        return Ok(n);
                    }
                    match self.rx.recv() {
                        Ok(bytes) => self.buf = Cursor::new(bytes),
                        Err(_) => return Ok(0),
                    }
                }
            }
        }
        let (tx, rx) = mpsc::channel();
        let mut framed = Vec::new();
        write_frame(&mut framed, first).unwrap();
        tx.send(framed).unwrap();
        (tx, ChanReader { rx, buf: Cursor::new(Vec::new()) })
    }
}
