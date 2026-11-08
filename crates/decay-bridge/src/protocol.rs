//! The native messaging protocol (API Design §4): one envelope for every message between
//! the extension, `decay-host` and the app, and the typed payloads.
//!
//! Payload field names are the extension's camelCase. Unknown fields are ignored, so an
//! older app keeps working with a newer extension (the `v` field marks breaking changes).

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Protocol version carried in every envelope.
pub const PROTOCOL_VERSION: u32 = 1;
/// The native messaging host name the extension connects to.
pub const HOST_NAME: &str = "com.decay.host";

/// Part of a payload split for the browser's 1 MB message limit (see [`crate::chunk`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chunk {
    pub seq: u32,
    pub total: u32,
    pub group_id: String,
}

/// Every message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub v: u32,
    /// Message id (UUID), echoed in replies as `replyTo`.
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk: Option<Chunk>,
    #[serde(default)]
    pub payload: Value,
}

/// A payload couldn't be built or read.
#[derive(Debug, thiserror::Error)]
#[error("bad {kind} payload: {source}")]
pub struct PayloadError {
    pub kind: String,
    #[source]
    pub source: serde_json::Error,
}

impl Envelope {
    /// A new message with a fresh id.
    pub fn new(kind: &str, payload: &impl Serialize) -> Result<Self, PayloadError> {
        let payload = serde_json::to_value(payload)
            .map_err(|source| PayloadError { kind: kind.to_owned(), source })?;
        Ok(Self {
            v: PROTOCOL_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            kind: kind.to_owned(),
            reply_to: None,
            chunk: None,
            payload,
        })
    }

    /// A reply to `self`.
    pub fn reply(&self, kind: &str, payload: &impl Serialize) -> Result<Self, PayloadError> {
        Ok(Self { reply_to: Some(self.id.clone()), ..Self::new(kind, payload)? })
    }

    /// An `error` reply (or a standalone error when `to` is `None`).
    pub fn error(to: Option<&Self>, code: ErrorCode, message: impl Into<String>) -> Self {
        let payload = ErrorPayload { code, message: message.into() };
        let mut env = Self::new(kinds::ERROR, &payload).unwrap_or_else(|_| Self {
            v: PROTOCOL_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            kind: kinds::ERROR.to_owned(),
            reply_to: None,
            chunk: None,
            payload: Value::Null,
        });
        env.reply_to = to.map(|t| t.id.clone());
        env
    }

    /// The payload as `T`.
    pub fn payload_as<T: DeserializeOwned>(&self) -> Result<T, PayloadError> {
        T::deserialize(&self.payload)
            .map_err(|source| PayloadError { kind: self.kind.clone(), source })
    }
}

/// Message types (`Envelope::kind`).
pub mod kinds {
    // Extension → core.
    pub const HELLO: &str = "hello";
    pub const SNAPSHOT: &str = "snapshot";
    pub const TAB_USED: &str = "tab.used";
    pub const TAB_CREATED: &str = "tab.created";
    pub const TAB_UPDATED: &str = "tab.updated";
    pub const TAB_REMOVED: &str = "tab.removed";
    pub const TAB_MOVED: &str = "tab.moved";
    pub const WINDOW_FOCUS: &str = "window.focus";
    pub const PROBE_RESULT: &str = "probe.result";
    pub const ACK: &str = "ack";
    pub const HEARTBEAT: &str = "heartbeat";
    pub const IMPORT_ARCHIVE: &str = "import.archive";
    // Core → extension.
    pub const WELCOME: &str = "welcome";
    pub const PROBE: &str = "probe";
    pub const TABS_CLOSE: &str = "tabs.close";
    pub const TABS_RESTORE: &str = "tabs.restore";
    pub const TABS_FOCUS: &str = "tabs.focus";
    pub const RESYNC: &str = "resync";
    pub const CONFIG: &str = "config";
    // Both ways.
    pub const ERROR: &str = "error";
}

/// Error codes (API Design §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    VersionMismatch,
    AppUnavailable,
    Unsafe,
    NotFound,
    Timeout,
    BadMessage,
    /// A code from a newer version.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub code: ErrorCode,
    #[serde(default)]
    pub message: String,
}

/// Which Chromium browser the extension runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserKind {
    Chrome,
    Edge,
    Brave,
    Arc,
    Chromium,
    /// Another Chromium browser (or a newer extension's value).
    #[serde(other)]
    Other,
}

/// `hello`: the first message of a connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub ext_version: String,
    pub browser: BrowserKind,
    /// Random UUID kept in `chrome.storage` (one per browser profile).
    pub profile_id: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Start the app if it isn't running. The extension sets this only when the browser
    /// starts or the user asks: reconnect attempts must not reopen an app the user quit.
    #[serde(default)]
    pub launch: bool,
}

/// Longest `hello.profileId` accepted (it's a UUID; it's repeated in every IPC frame).
pub const MAX_PROFILE_ID_LEN: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WelcomeConfig {
    pub probe_enabled: bool,
    pub heartbeat_sec: u32,
}

/// `welcome`: the app's answer to `hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Welcome {
    pub core_version: String,
    pub min_ext_version: String,
    pub mode: decay_core::Mode,
    pub config: WelcomeConfig,
}

/// A tab as reported by the extension.
#[allow(clippy::struct_excessive_bools, reason = "mirrors the extension's JSON")]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabInfo {
    pub tab_id: i64,
    pub window_id: i64,
    pub index: i64,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fav_icon_url: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub audible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted_info: Option<MutedInfo>,
    #[serde(default)]
    pub discarded: bool,
    /// −1 when not in a group.
    #[serde(default = "no_group")]
    pub group_id: i64,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub status: TabStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_accessed: Option<f64>,
}

const fn no_group() -> i64 {
    -1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutedInfo {
    pub muted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TabStatus {
    Loading,
    #[default]
    Complete,
    Unloaded,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowState {
    #[default]
    Normal,
    Minimized,
    Maximized,
    Fullscreen,
    #[serde(rename = "locked-fullscreen")]
    LockedFullscreen,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub id: i64,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub state: WindowState,
    #[serde(default)]
    pub incognito: bool,
    #[serde(default)]
    pub tabs: Vec<TabInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupInfo {
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub color: String,
    pub window_id: i64,
}

/// `snapshot`: every window and group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Snapshot {
    #[serde(default)]
    pub windows: Vec<WindowInfo>,
    #[serde(default)]
    pub groups: Vec<GroupInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UseSignal {
    Activated,
    Visible,
    Audible,
    Typed,
    #[serde(other)]
    Unknown,
}

/// `tab.used`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabUsed {
    pub tab_id: i64,
    pub window_id: i64,
    pub at: f64,
    pub signal: UseSignal,
}

/// `tab.updated`: the changed fields (`chrome.tabs.onUpdated`'s `changeInfo`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabUpdated {
    pub tab_id: i64,
    #[serde(default)]
    pub changes: serde_json::Map<String, Value>,
}

/// `tab.removed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabRemoved {
    pub tab_id: i64,
    /// Closed by the user (not by Decay): the item is deleted, not archived.
    #[serde(default)]
    pub by_user: bool,
}

/// `tab.moved`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabMoved {
    pub tab_id: i64,
    pub window_id: i64,
    pub index: i64,
}

/// `window.focus`: −1 when no browser window has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowFocus {
    pub window_id: i64,
}

/// `probe.result`.
#[allow(clippy::struct_excessive_bools, reason = "mirrors the extension's JSON")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub tab_id: i64,
    pub dirty_form: bool,
    pub before_unload: bool,
    pub uploading: bool,
    /// The page couldn't be checked (the 2× threshold applies).
    #[serde(default)]
    pub unknown: bool,
}

/// `ack`: the reply to a command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ack {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

/// `heartbeat`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Heartbeat {
    pub at: f64,
}

/// `probe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub tab_ids: Vec<i64>,
}

/// `tabs.close`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabsClose {
    pub sweep_id: String,
    pub tab_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreGroup {
    pub title: String,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreItem {
    pub item_id: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<RestoreGroup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
}

/// `tabs.restore`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabsRestore {
    pub items: Vec<RestoreItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_window: Option<bool>,
}

/// `tabs.focus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabsFocus {
    pub tab_id: i64,
}

/// `config`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub probe_enabled: bool,
    #[serde(default)]
    pub excluded_domains: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn envelopes_use_the_extension_json_shape() {
        let hello = Hello {
            ext_version: "0.2.0".into(),
            browser: BrowserKind::Brave,
            profile_id: "p1".into(),
            capabilities: vec!["tabGroups".into()],
            launch: false,
        };
        let env = Envelope::new(kinds::HELLO, &hello).unwrap();
        let json = serde_json::to_value(&env).unwrap();
        assert_eq!(json["v"], 1);
        assert_eq!(json["type"], "hello");
        assert_eq!(json["payload"]["extVersion"], "0.2.0");
        assert_eq!(json["payload"]["browser"], "brave");
        assert!(json.get("replyTo").is_none() && json.get("chunk").is_none());
        assert_eq!(env.payload_as::<Hello>().unwrap(), hello);

        let err = Envelope::error(Some(&env), ErrorCode::AppUnavailable, "not running");
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["replyTo"], env.id.as_str());
        assert_eq!(json["payload"], json!({"code": "APP_UNAVAILABLE", "message": "not running"}));
    }

    #[test]
    fn reads_tabs_leniently() {
        let tab: TabInfo = serde_json::from_value(json!({
            "tabId": 5, "windowId": 1, "index": 0, "url": "https://a.com/", "title": "A",
            "pinned": false, "audible": true, "discarded": false, "active": true,
            "status": "complete", "lastAccessed": 1_793_750_400_000.5, "somethingNew": 1
        }))
        .unwrap();
        assert_eq!(tab.group_id, -1);
        assert!(tab.audible);
        assert_eq!(tab.last_accessed, Some(1_793_750_400_000.5));
        let removed: TabRemoved = serde_json::from_value(json!({"tabId": 5})).unwrap();
        assert!(!removed.by_user);
        // Values from a newer extension don't break the message.
        let tab: TabInfo = serde_json::from_value(
            json!({"tabId": 1, "windowId": 1, "index": 0, "status": "frozen"}),
        )
        .unwrap();
        assert_eq!(tab.status, TabStatus::Unknown);
        let err: ErrorPayload = serde_json::from_value(json!({"code": "QUOTA"})).unwrap();
        assert_eq!(err.code, ErrorCode::Unknown);
        let hello: Hello = serde_json::from_value(
            json!({"extVersion": "1", "browser": "vivaldi", "profileId": "p"}),
        )
        .unwrap();
        assert_eq!((hello.browser, hello.launch), (BrowserKind::Other, false));
    }
}
