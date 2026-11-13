//! The engine's threads (Software Design §4.6 runtime): the IPC server's accept loop, one
//! reader thread per connection, and one engine thread that owns the [`Engine`] (and so
//! the store), handles every message in arrival order and polls the timers every second.
//!
//! Plain threads and channels: the store is synchronous and the engine is one state
//! machine, so an async runtime would add nothing here (the Tauri shell, P1.7, keeps its
//! own).
//!
//! A connection the engine closes (`Output::Close`) is forgotten: the engine ignores its
//! messages, and the host process ends when its browser port closes.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use decay_bridge::ipc::{Endpoint, IpcServer, IpcWriter, connect};

use crate::engine::{ConnId, Engine, EngineEvent, Input, Output};

/// Unix epoch milliseconds now.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// How often the engine thread runs the timers.
const POLL_EVERY: Duration = Duration::from_secs(1);

/// Work for the engine thread from outside (the UI later; the dev tool now).
pub type Call = Box<dyn FnOnce(&mut Engine, i64) -> Vec<Output> + Send>;

enum Msg {
    NewConn(ConnId, IpcWriter),
    Input(Input),
    Call(Call),
    Stop,
}

/// The running engine.
#[derive(Debug)]
pub struct Runtime {
    tx: Sender<Msg>,
    engine: Option<JoinHandle<Engine>>,
    accept: Option<JoinHandle<()>>,
    stopping: Arc<AtomicBool>,
    endpoint: Endpoint,
}

impl std::fmt::Debug for Msg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NewConn(..) => "NewConn",
            Self::Input(_) => "Input",
            Self::Call(_) => "Call",
            Self::Stop => "Stop",
        })
    }
}

fn apply(
    outputs: Vec<Output>,
    writers: &mut HashMap<ConnId, IpcWriter>,
    events: &mut dyn FnMut(EngineEvent),
) {
    for out in outputs {
        match out {
            Output::Send(conn, env) => {
                let failed = writers.get_mut(&conn).is_some_and(|w| w.write_json(&env).is_err());
                if failed {
                    // The reader thread notices the closed connection and reports it.
                    tracing::warn!(conn, "couldn't write to the host");
                    writers.remove(&conn);
                }
            }
            Output::Close(conn) => {
                writers.remove(&conn);
            }
            Output::Event(event) => events(event),
        }
    }
}

impl Runtime {
    /// Starts serving `server` with `engine`. `events` runs on the engine thread.
    pub fn start(
        server: IpcServer,
        mut engine: Engine,
        mut events: impl FnMut(EngineEvent) + Send + 'static,
    ) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Msg>();
        let stopping = Arc::new(AtomicBool::new(false));
        let endpoint = server.endpoint().clone();

        let engine_thread =
            std::thread::Builder::new().name("decay-engine".into()).spawn(move || {
                let mut writers: HashMap<ConnId, IpcWriter> = HashMap::new();
                let mut last_poll = 0i64;
                loop {
                    let msg = match rx.recv_timeout(POLL_EVERY) {
                        Ok(msg) => Some(msg),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => break,
                    };
                    let now = now_ms();
                    let outputs = match msg {
                        Some(Msg::Stop) => break,
                        Some(Msg::NewConn(id, writer)) => {
                            writers.insert(id, writer);
                            engine.handle(Input::Connected(id), now)
                        }
                        Some(Msg::Input(input)) => {
                            if let Input::Disconnected(id) = &input {
                                writers.remove(id);
                            }
                            engine.handle(input, now)
                        }
                        Some(Msg::Call(call)) => call(&mut engine, now),
                        None => Vec::new(),
                    };
                    apply(outputs, &mut writers, &mut events);
                    // Timers run even while messages keep arriving.
                    if now - last_poll >= 1000 || now < last_poll {
                        last_poll = now;
                        let outputs = engine.poll(now);
                        apply(outputs, &mut writers, &mut events);
                    }
                }
                engine
            })?;

        let accept_tx = tx.clone();
        let accept_stop = Arc::clone(&stopping);
        let accept_thread =
            std::thread::Builder::new().name("decay-accept".into()).spawn(move || {
                let next = AtomicU64::new(1);
                loop {
                    let accepted = server.accept();
                    if accept_stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let (mut reader, writer) = match accepted {
                        Ok(pair) => pair,
                        Err(e) => {
                            tracing::warn!("accept failed: {e}");
                            std::thread::sleep(Duration::from_millis(100));
                            continue;
                        }
                    };
                    let id = next.fetch_add(1, Ordering::Relaxed);
                    if accept_tx.send(Msg::NewConn(id, writer)).is_err() {
                        break;
                    }
                    let tx = accept_tx.clone();
                    let spawned = std::thread::Builder::new()
                        .name(format!("decay-conn-{id}"))
                        .spawn(move || {
                            loop {
                                match reader.read_host_frame() {
                                    Ok(Some(frame)) => {
                                        if tx
                                            .send(Msg::Input(Input::Frame(id, Box::new(frame))))
                                            .is_err()
                                        {
                                            return;
                                        }
                                    }
                                    // A frame that isn't valid JSON was read whole: skip it.
                                    Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                                        tracing::warn!(conn = id, "unreadable frame: {e}");
                                    }
                                    // Closed, or the connection failed.
                                    Ok(None) | Err(_) => break,
                                }
                            }
                            let _ = tx.send(Msg::Input(Input::Disconnected(id)));
                        });
                    if let Err(e) = spawned {
                        tracing::error!("couldn't start a connection thread: {e}");
                        let _ = accept_tx.send(Msg::Input(Input::Disconnected(id)));
                    }
                }
                drop(server);
            })?;

        Ok(Self {
            tx,
            engine: Some(engine_thread),
            accept: Some(accept_thread),
            stopping,
            endpoint,
        })
    }

    /// Runs `call` on the engine thread (its outputs are applied like any other).
    pub fn call(&self, call: Call) -> bool {
        self.tx.send(Msg::Call(call)).is_ok()
    }

    /// Stops the threads and returns the engine (its store is still open).
    pub fn stop(mut self) -> Option<Engine> {
        self.stopping.store(true, Ordering::SeqCst);
        let _ = self.tx.send(Msg::Stop);
        // Wake the accept loop so it sees the flag and drops the server (removing the socket).
        let _ = connect(&self.endpoint);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        self.engine.take().and_then(|t| t.join().ok())
    }
}
