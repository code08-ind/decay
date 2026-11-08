//! Host ⇄ app IPC (API Design §5, Software Design §6): a local socket, framed like native
//! messaging. The host sends [`HostFrame`]s (each extension message with its connection's
//! metadata); the app sends plain envelopes back.
//!
//! - macOS / Unix: `<support dir>/run/decay.sock`. The `run` folder must belong to this
//!   user and is `0700`, the socket `0600`. Both ends check that the other process runs as
//!   the same user.
//! - Windows: a named pipe with a random name chosen each time the app starts, published
//!   in `<support dir>\run\pipe-name` (the user's own profile folder). A fixed name
//!   could be created first by another user to receive this user's tabs ("pipe
//!   squatting"); a random one can only be found by someone who can read the user's
//!   profile. The pipe's access list allows only its owner and SYSTEM. (The doc's
//!   `decay-<user SID>` would need raw Win32 calls, which `forbid(unsafe_code)` rules out,
//!   and wouldn't stop squatting.) If the app runs elevated, its pipe's owner is
//!   Administrators and an unelevated host is refused: the app must not run elevated.

use std::ffi::OsString;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{ListenerOptions, Name, Stream};
use serde::{Deserialize, Serialize};

use crate::frame::{MAX_IPC_FRAME, read_frame, write_frame};
use crate::protocol::{BrowserKind, Envelope};
use crate::text::decode_envelope;

/// Overrides the IPC endpoint (a socket path on Unix, a pipe name on Windows). Debug
/// builds only (tests, development next to an installed app): in a release build anything
/// that can change the browser's environment could otherwise redirect the host.
pub const ENDPOINT_ENV: &str = "DECAY_IPC_ENDPOINT";
/// Overrides the app's support folder. Debug builds only, like [`ENDPOINT_ENV`].
pub const SUPPORT_DIR_ENV: &str = "DECAY_SUPPORT_DIR";

fn env_override(name: &str) -> Option<OsString> {
    if cfg!(debug_assertions) { std::env::var_os(name) } else { None }
}

/// The app's per-user data folder (DB Design §1): `~/Library/Application Support/Decay`
/// on macOS, `%APPDATA%\Decay` on Windows, `$XDG_DATA_HOME/decay` elsewhere.
pub fn support_dir() -> Option<PathBuf> {
    if let Some(dir) = env_override(SUPPORT_DIR_ENV) {
        return Some(PathBuf::from(dir));
    }
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/Application Support/Decay"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Decay"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".local/share")))
            .map(|d| d.join("decay"))
    }
}

/// Where a Windows app publishes its pipe name.
pub fn pipe_name_file(support_dir: &Path) -> PathBuf {
    support_dir.join("run").join("pipe-name")
}

fn no_support_dir() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "no home folder")
}

/// Where the app listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A Unix domain socket.
    Socket(PathBuf),
    /// A Windows named pipe (the name after `\\.\pipe\`).
    Pipe(String),
}

impl Endpoint {
    /// Where the host finds the running app: the socket path, or on Windows the pipe name
    /// the app published (`NotFound` when the app isn't running).
    pub fn default_for_user() -> io::Result<Self> {
        let from_env = env_override(ENDPOINT_ENV);
        if cfg!(windows) {
            if let Some(name) = from_env {
                return Ok(Self::Pipe(name.to_string_lossy().into_owned()));
            }
            let file = pipe_name_file(&support_dir().ok_or_else(no_support_dir)?);
            let name = std::fs::read_to_string(file)?;
            let name = name.trim();
            let valid = name.starts_with("decay-")
                && name.len() <= 64
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
            if !valid {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "bad pipe name file"));
            }
            Ok(Self::Pipe(name.to_owned()))
        } else {
            if let Some(path) = from_env {
                return Ok(Self::Socket(PathBuf::from(path)));
            }
            let dir = support_dir().ok_or_else(no_support_dir)?;
            Ok(Self::Socket(dir.join("run").join("decay.sock")))
        }
    }

    /// Where the app should listen: the socket path, or on Windows a new random pipe name
    /// (publish it with [`IpcServer::publish`]).
    pub fn new_for_app() -> io::Result<Self> {
        if cfg!(windows) {
            if let Some(name) = env_override(ENDPOINT_ENV) {
                return Ok(Self::Pipe(name.to_string_lossy().into_owned()));
            }
            Ok(Self::Pipe(format!("decay-{}", uuid::Uuid::new_v4().simple())))
        } else {
            Self::default_for_user()
        }
    }

    fn name(&self) -> io::Result<Name<'_>> {
        match self {
            Self::Socket(path) => {
                path.as_path().to_fs_name::<interprocess::local_socket::GenericFilePath>()
            }
            Self::Pipe(name) => {
                name.as_str().to_ns_name::<interprocess::local_socket::GenericNamespaced>()
            }
        }
    }
}

/// Which extension instance a message came from (added by the host).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnInfo {
    pub browser: BrowserKind,
    pub profile_id: String,
    /// The host process (one per extension connection).
    pub pid: u32,
    /// The calling extension, e.g. `chrome-extension://<id>/`.
    pub origin: String,
}

/// What the host sends the app: an extension message with its connection metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostFrame {
    pub conn: ConnInfo,
    pub msg: Envelope,
}

/// One connection, shared by its reading and writing halves (`&Stream` reads and writes).
/// The connection closes when both halves are dropped.
#[derive(Debug, Clone)]
struct Shared(Arc<Stream>);

impl Read for Shared {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&*self.0).read(buf)
    }
}

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self.0).write(buf)
    }
    /// Writes reach the other side as they're made. Flushing the stream itself would, on a
    /// Windows pipe, wait until the other side has *read* everything (`FlushFileBuffers`),
    /// turning each frame into a round trip.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The receiving half of a connection.
#[derive(Debug)]
pub struct IpcReader(BufReader<Shared>);

/// The sending half of a connection.
#[derive(Debug)]
pub struct IpcWriter(BufWriter<Shared>);

impl IpcReader {
    /// The next frame's JSON, or `None` when the other side closed the connection.
    pub fn read_raw(&mut self) -> io::Result<Option<Vec<u8>>> {
        read_frame(&mut self.0, MAX_IPC_FRAME)
    }

    /// The next envelope (app → host direction).
    pub fn read_envelope(&mut self) -> io::Result<Option<Envelope>> {
        self.read_raw()?
            .map(|b| decode_envelope(&b).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)))
            .transpose()
    }

    /// The next host frame (host → app direction).
    pub fn read_host_frame(&mut self) -> io::Result<Option<HostFrame>> {
        self.read_raw()?
            .map(|b| {
                let text = String::from_utf8_lossy(&b);
                serde_json::from_str(&crate::text::sanitize_surrogates(&text))
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
            })
            .transpose()
    }
}

impl IpcWriter {
    pub fn write_json(&mut self, value: &impl Serialize) -> io::Result<()> {
        let body =
            serde_json::to_vec(value).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.write_raw(&body)
    }

    /// Writes one frame of JSON that is already serialised.
    pub fn write_raw(&mut self, json: &[u8]) -> io::Result<()> {
        if json.len() > MAX_IPC_FRAME {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "frame over the IPC limit"));
        }
        write_frame(&mut self.0, json)
    }
}

/// A host frame built around an extension message's JSON as received, without parsing it
/// into a tree (the host stays a cheap relay, and the message is forwarded byte for byte).
pub fn host_frame_json(conn: &ConnInfo, msg_json: &str) -> io::Result<Vec<u8>> {
    let conn =
        serde_json::to_vec(conn).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut out = Vec::with_capacity(conn.len() + msg_json.len() + 16);
    out.extend_from_slice(b"{\"conn\":");
    out.extend_from_slice(&conn);
    out.extend_from_slice(b",\"msg\":");
    out.extend_from_slice(msg_json.as_bytes());
    out.push(b'}');
    Ok(out)
}

#[cfg(unix)]
fn my_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

fn halves(stream: Stream) -> (IpcReader, IpcWriter) {
    let shared = Shared(Arc::new(stream));
    (IpcReader(BufReader::new(shared.clone())), IpcWriter(BufWriter::new(shared)))
}

/// Connects to the app (used by the host). On Unix, the listening process must run as
/// this user: a socket put there by anyone else is refused.
pub fn connect(endpoint: &Endpoint) -> io::Result<(IpcReader, IpcWriter)> {
    let stream = Stream::connect(endpoint.name()?)?;
    #[cfg(unix)]
    {
        use interprocess::local_socket::traits::StreamCommon;
        if stream.peer_creds()?.euid() != Some(my_uid()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "socket owned by another user",
            ));
        }
    }
    Ok(halves(stream))
}

/// The app's listening socket.
#[derive(Debug)]
pub struct IpcServer {
    listener: interprocess::local_socket::Listener,
    endpoint: Endpoint,
    /// The socket file's (device, inode), so shutdown removes only our own.
    #[cfg(unix)]
    socket_id: (u64, u64),
    /// The published pipe-name file (Windows), removed on shutdown.
    published: Option<PathBuf>,
}

/// Creates `run/` for the socket: a real folder, owned by this user, closed to others.
#[cfg(unix)]
fn private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "socket folder isn't a folder",
        ));
    }
    if meta.uid() != my_uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "socket folder owned by another user",
        ));
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

impl IpcServer {
    /// Listens on `endpoint`. On Unix, a stale socket (or a stray file or link) left at the
    /// path is replaced; a live one (another app instance) makes this fail with
    /// `AddrInUse`. Call it once per app: the app's single-owner database lock (P1.2)
    /// keeps two instances from racing here.
    pub fn bind(endpoint: Endpoint) -> io::Result<Self> {
        #[cfg(unix)]
        {
            let Endpoint::Socket(path) = &endpoint else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "named pipes are Windows-only",
                ));
            };
            let dir = path.parent().ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
            private_dir(dir)?;
            match std::fs::symlink_metadata(path) {
                Ok(meta) if meta.is_dir() => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "socket path is a folder",
                    ));
                }
                Ok(_) => {
                    if Stream::connect(endpoint.name()?).is_ok() {
                        return Err(io::Error::new(
                            io::ErrorKind::AddrInUse,
                            "the app is already running",
                        ));
                    }
                    std::fs::remove_file(path)?; // a link is removed, not its target
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        // `reclaim_name(false)`: our `Drop` removes the socket, and only if it's still ours.
        let options = ListenerOptions::new().name(endpoint.name()?).reclaim_name(false);
        #[cfg(windows)]
        let options = {
            use interprocess::os::windows::local_socket::ListenerOptionsExt;
            use interprocess::os::windows::security_descriptor::SecurityDescriptor;
            // Protected DACL: full access for the pipe's owner (this user) and SYSTEM only.
            let sddl = widestring::U16CString::from_str("D:P(A;;GA;;;OW)(A;;GA;;;SY)")
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            options.security_descriptor(SecurityDescriptor::deserialize(&sddl)?)
        };
        let listener = options.create_sync()?;
        #[cfg(unix)]
        let socket_id = {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let Endpoint::Socket(path) = &endpoint else { unreachable!() };
            // macOS can't set a socket's mode at creation; the private folder covers the
            // moment in between.
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            let meta = std::fs::symlink_metadata(path)?;
            (meta.dev(), meta.ino())
        };
        Ok(Self {
            listener,
            endpoint,
            #[cfg(unix)]
            socket_id,
            published: None,
        })
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Windows: writes the pipe name where the host looks for it
    /// ([`pipe_name_file`]); removed again on shutdown. No-op for a Unix socket.
    pub fn publish(&mut self, support_dir: &Path) -> io::Result<()> {
        let Endpoint::Pipe(name) = &self.endpoint else { return Ok(()) };
        let file = pipe_name_file(support_dir);
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = file.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
        std::fs::write(&tmp, name.as_bytes())?;
        std::fs::rename(&tmp, &file)?;
        self.published = Some(file);
        Ok(())
    }

    /// The next connection from a process of this user. Connections from other users
    /// (possible only if they get past the folder permissions) are dropped.
    pub fn accept(&self) -> io::Result<(IpcReader, IpcWriter)> {
        loop {
            let stream = self.listener.accept()?;
            #[cfg(unix)]
            {
                use interprocess::local_socket::traits::StreamCommon;
                if stream.peer_creds().ok().and_then(|c| c.euid()) != Some(my_uid()) {
                    continue;
                }
            }
            return Ok(halves(stream));
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Endpoint::Socket(path) = &self.endpoint {
            use std::os::unix::fs::MetadataExt;
            let ours =
                std::fs::symlink_metadata(path).is_ok_and(|m| (m.dev(), m.ino()) == self.socket_id);
            if ours {
                let _ = std::fs::remove_file(path);
            }
        }
        if let (Some(file), Endpoint::Pipe(name)) = (&self.published, &self.endpoint)
            && std::fs::read_to_string(file).is_ok_and(|n| n.trim() == name)
        {
            let _ = std::fs::remove_file(file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Hello, kinds};
    use std::thread;

    pub(crate) fn test_endpoint(dir: &Path) -> Endpoint {
        if cfg!(windows) {
            Endpoint::Pipe(format!("decay-test-{}", uuid::Uuid::new_v4()))
        } else {
            Endpoint::Socket(dir.join("run").join("decay.sock"))
        }
    }

    fn hello_frame() -> HostFrame {
        let hello = Hello {
            ext_version: "1.0.0".into(),
            browser: BrowserKind::Chrome,
            profile_id: "p".into(),
            capabilities: vec![],
            launch: false,
        };
        HostFrame {
            conn: ConnInfo {
                browser: BrowserKind::Chrome,
                profile_id: "p".into(),
                pid: 42,
                origin: "chrome-extension://abc/".into(),
            },
            msg: Envelope::new(kinds::HELLO, &hello).unwrap(),
        }
    }

    #[test]
    fn host_and_app_exchange_frames() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = test_endpoint(dir.path());
        let server = IpcServer::bind(endpoint.clone()).unwrap();
        let app = thread::spawn(move || {
            let (mut rx, mut tx) = server.accept().unwrap();
            let frame = rx.read_host_frame().unwrap().unwrap();
            let reply = frame.msg.reply(kinds::WELCOME, &serde_json::json!({"ok": true})).unwrap();
            tx.write_json(&reply).unwrap();
            assert_eq!(rx.read_host_frame().unwrap(), None, "host closed");
            frame
        });
        let (mut rx, mut tx) = connect(&endpoint).unwrap();
        let sent = hello_frame();
        tx.write_json(&sent).unwrap();
        let reply = rx.read_envelope().unwrap().unwrap();
        assert_eq!(reply.reply_to.as_deref(), Some(sent.msg.id.as_str()));
        drop((rx, tx));
        assert_eq!(app.join().unwrap(), sent);
    }

    #[cfg(unix)]
    #[test]
    fn the_socket_is_private_and_single_instance() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let endpoint = test_endpoint(dir.path());
        let Endpoint::Socket(path) = &endpoint else { unreachable!() };
        let server = IpcServer::bind(endpoint.clone()).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        assert_eq!(mode(path), 0o600);
        let second = IpcServer::bind(endpoint.clone()).unwrap_err();
        assert_eq!(second.kind(), io::ErrorKind::AddrInUse);
        drop(server);
        assert!(!path.exists(), "removed on shutdown");
        // A stale socket file (crashed app) doesn't block the next start.
        std::fs::write(path, b"").unwrap();
        IpcServer::bind(endpoint).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_linked_socket_folder_and_cleans_up_stray_links() {
        let dir = tempfile::tempdir().unwrap();
        // `run` as a link to another folder: refused.
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.path().join("run")).unwrap();
        let err = IpcServer::bind(test_endpoint(dir.path())).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);

        // A dangling link at the socket path is removed (not followed), then bound.
        let dir = tempfile::tempdir().unwrap();
        let endpoint = test_endpoint(dir.path());
        let Endpoint::Socket(path) = &endpoint else { unreachable!() };
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let target = dir.path().join("precious.txt");
        std::os::unix::fs::symlink(&target, path).unwrap();
        let server = IpcServer::bind(endpoint.clone()).unwrap();
        assert!(!target.exists(), "the link's target was never created");

        // Shutdown removes only our own socket, not one that replaced it.
        std::fs::remove_file(path).unwrap();
        std::fs::write(path, b"someone else's").unwrap();
        drop(server);
        assert!(path.exists());
    }

    #[cfg(windows)]
    #[test]
    fn the_host_finds_the_published_random_pipe() {
        let dir = tempfile::tempdir().unwrap();
        let name = format!("decay-{}", uuid::Uuid::new_v4().simple());
        let mut server = IpcServer::bind(Endpoint::Pipe(name.clone())).unwrap();
        server.publish(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(pipe_name_file(dir.path())).unwrap(), name);
        drop(server);
        assert!(!pipe_name_file(dir.path()).exists());
    }

    #[test]
    fn host_frames_wrap_the_message_unchanged() {
        let conn = hello_frame().conn;
        let msg = r#"{"v":1,"id":"x","type":"heartbeat","payload":{"at":1}}"#;
        let bytes = host_frame_json(&conn, msg).unwrap();
        let frame: HostFrame = serde_json::from_slice(&bytes).unwrap();
        assert_eq!((frame.conn, frame.msg.kind.as_str()), (conn, "heartbeat"));
    }

    #[test]
    fn connecting_without_an_app_fails() {
        let dir = tempfile::tempdir().unwrap();
        assert!(connect(&test_endpoint(dir.path())).is_err());
    }
}
