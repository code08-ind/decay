//! The runtime over a real local socket / pipe: a fake `decay-host` connects, says hello,
//! and gets `welcome` and `config` back; a snapshot reaches the store; a disconnect is
//! noticed; `stop` hands the engine back.

use std::time::{Duration, Instant};

use decay_bridge::ipc::{ConnInfo, Endpoint, HostFrame, IpcServer, connect};
use decay_bridge::protocol::BrowserKind;
use decay_bridge::{Envelope, kinds};
use decay_engine::runtime::{Runtime, now_ms};
use decay_engine::{Engine, EngineConfig, EngineEvent};
use decay_store::{DbKey, Store, StorePaths};
use serde_json::json;

fn endpoint(dir: &std::path::Path) -> Endpoint {
    if cfg!(windows) {
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        Endpoint::Pipe(format!("decay-test-{nanos:x}"))
    } else {
        Endpoint::Socket(dir.join("run").join("decay.sock"))
    }
}

fn frame(msg: &Envelope) -> HostFrame {
    HostFrame {
        conn: ConnInfo {
            browser: BrowserKind::Chrome,
            profile_id: "p1".into(),
            pid: 42,
            origin: "chrome-extension://x/".into(),
        },
        msg: msg.clone(),
    }
}

#[test]
fn serves_a_host_connection_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) =
        Store::open(StorePaths::in_dir(dir.path()), DbKey::from_bytes([3; 32]), now_ms()).unwrap();
    let engine = Engine::new(store, EngineConfig::default(), now_ms()).unwrap();
    let ep = endpoint(dir.path());
    let server = IpcServer::bind(ep.clone()).unwrap();
    let (events_tx, events_rx) = std::sync::mpsc::channel();
    let (status_tx, status_rx) = std::sync::mpsc::channel();
    let runtime =
        Runtime::start(server, engine, Box::new(decay_platform::NullPlatform), move |e| {
            // Status updates are checked on their own.
            if let EngineEvent::StatusChanged(s) = e {
                let _ = status_tx.send(*s);
            } else {
                let _ = events_tx.send(e);
            }
        })
        .unwrap();

    let (mut reader, mut writer) = connect(&ep).unwrap();
    let hello = Envelope::new(
        kinds::HELLO,
        &json!({"extVersion": "0.1.0", "browser": "chrome", "profileId": "p1", "session": "s"}),
    )
    .unwrap();
    writer.write_json(&frame(&hello)).unwrap();
    let welcome = reader.read_envelope().unwrap().unwrap();
    assert_eq!(
        (welcome.kind.as_str(), welcome.reply_to.as_deref()),
        ("welcome", Some(hello.id.as_str()))
    );
    let config = reader.read_envelope().unwrap().unwrap();
    assert_eq!(config.kind, kinds::CONFIG);
    let mut ack = Envelope::new(kinds::ACK, &json!({"ok": true})).unwrap();
    ack.reply_to = Some(config.id.clone());
    writer.write_json(&frame(&ack)).unwrap();
    let snap = Envelope::new(
        kinds::SNAPSHOT,
        &json!({"windows": [{"id": 1, "focused": true, "state": "normal", "incognito": false,
            "tabs": [{"tabId": 5, "windowId": 1, "index": 0, "url": "https://a.example/", "title": "A"}]}],
            "groups": []}),
    )
    .unwrap();
    writer.write_json(&frame(&snap)).unwrap();
    assert!(matches!(
        events_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        EngineEvent::AdapterConnected { .. }
    ));
    // The status follows: one browser, one tab.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let s = status_rx.recv_timeout(deadline - Instant::now()).unwrap();
        if s.live_tabs == 1 && s.browsers.len() == 1 {
            break;
        }
    }
    drop((reader, writer));
    assert!(matches!(
        events_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        EngineEvent::AdapterDisconnected { .. }
    ));

    let started = Instant::now();
    let engine = runtime.stop().expect("the engine comes back");
    assert!(started.elapsed() < Duration::from_secs(5));
    let live = engine.store().live_items().unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].external_id.as_deref(), Some("p1:5"));
}
