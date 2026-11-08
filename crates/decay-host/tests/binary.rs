//! The real `decay-host` executable, run the way a browser runs it: origin as the first
//! argument, native messaging frames on stdin / stdout.

use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;

use decay_bridge::frame::{read_envelope, write_frame};
use decay_bridge::ipc::{ENDPOINT_ENV, Endpoint, IpcServer, SUPPORT_DIR_ENV};
use decay_bridge::protocol::{ErrorCode, ErrorPayload};
use decay_bridge::{Envelope, kinds};
use decay_host::config::{HostConfig, origin_of};
use serde_json::json;

const ID: &str = "abcdefghijklmnopabcdefghijklmnop";

fn endpoint(dir: &std::path::Path) -> (Endpoint, String) {
    if cfg!(windows) {
        let name = format!("decay-bin-test-{}", std::process::id());
        (Endpoint::Pipe(name.clone()), name)
    } else {
        let path = dir.join("run").join("decay.sock");
        (Endpoint::Socket(path.clone()), path.display().to_string())
    }
}

fn hello() -> Vec<u8> {
    let env = Envelope::new(
        kinds::HELLO,
        &json!({"extVersion": "1.0.0", "browser": "chrome", "profileId": "p", "capabilities": [],
                "launch": true}),
    )
    .unwrap();
    let mut buf = Vec::new();
    write_frame(&mut buf, &serde_json::to_vec(&env).unwrap()).unwrap();
    buf
}

fn host(dir: &std::path::Path, endpoint: &str, origin: &str) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_decay-host"));
    cmd.arg(origin)
        .env(SUPPORT_DIR_ENV, dir)
        .env(ENDPOINT_ENV, endpoint)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    cmd
}

#[test]
fn relays_hello_and_welcome_through_the_binary() {
    let dir = tempfile::tempdir().unwrap();
    HostConfig { allowed_origins: vec![origin_of(ID)], app_path: None }.save(dir.path()).unwrap();
    let (ep, ep_text) = endpoint(dir.path());
    let server = IpcServer::bind(ep).unwrap();
    let app = thread::spawn(move || {
        let (mut rx, mut tx) = server.accept().unwrap();
        let frame = rx.read_host_frame().unwrap().unwrap();
        tx.write_json(&frame.msg.reply(kinds::WELCOME, &json!({"hi": true})).unwrap()).unwrap();
        frame
    });
    let mut child = host(dir.path(), &ep_text, &origin_of(ID)).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(&hello()).unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let welcome = read_envelope(&mut stdout, usize::MAX).unwrap().unwrap();
    assert_eq!(welcome.kind, "welcome");
    let frame = app.join().unwrap();
    assert_eq!(frame.conn.origin, origin_of(ID));
    drop(stdin); // the browser closes the port
    assert!(child.wait().unwrap().success());
}

#[test]
fn an_unlisted_extension_gets_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (_, ep_text) = endpoint(dir.path());
    let mut child = host(dir.path(), &ep_text, &origin_of(ID)).spawn().unwrap(); // no config
    child.stdin.take().unwrap().write_all(&hello()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(!out.status.success());
    assert_eq!(out.stdout.len(), 0);
}

#[test]
fn reports_the_app_as_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    HostConfig { allowed_origins: vec![origin_of(ID)], app_path: None }.save(dir.path()).unwrap();
    let (_, ep_text) = endpoint(dir.path());
    let mut cmd = host(dir.path(), &ep_text, &origin_of(ID));
    // Don't start a real app from the test: on macOS `open -b` would look for one.
    cmd.env("PATH", "");
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(&hello()).unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let reply = read_envelope(&mut stdout, usize::MAX).unwrap().unwrap();
    assert_eq!(reply.payload_as::<ErrorPayload>().unwrap().code, ErrorCode::AppUnavailable);
    assert!(child.wait().unwrap().success());
}

#[test]
fn stdout_carries_only_protocol_frames_even_for_garbage() {
    let dir = tempfile::tempdir().unwrap();
    HostConfig { allowed_origins: vec![origin_of(ID)], app_path: None }.save(dir.path()).unwrap();
    let (_, ep_text) = endpoint(dir.path());
    let mut child = host(dir.path(), &ep_text, &origin_of(ID)).spawn().unwrap();
    let mut garbage = Vec::new();
    write_frame(&mut garbage, b"\xff\xfe not json at all").unwrap();
    child.stdin.take().unwrap().write_all(&garbage).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(!out.status.success());
    let mut r = std::io::Cursor::new(out.stdout);
    let reply = read_envelope(&mut r, usize::MAX).unwrap().unwrap();
    assert_eq!(reply.payload_as::<ErrorPayload>().unwrap().code, ErrorCode::BadMessage);
    assert_eq!(read_envelope(&mut r, usize::MAX).unwrap(), None, "nothing else on stdout");
}
