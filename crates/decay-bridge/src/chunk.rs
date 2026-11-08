//! Chunking (API Design §4.3). A message whose JSON is over the 1 MB limit is serialised,
//! cut into pieces, and each piece is sent as a string payload:
//!
//! ```json
//! {"v":1,"id":"<new>","type":"<original type>","chunk":{"seq":0,"total":3,"groupId":"<uuid>"},
//!  "payload":"<piece of the original message's JSON>"}
//! ```
//!
//! The receiver joins the pieces in `seq` order and parses the original envelope. Pieces
//! are cut at character boundaries so that each chunk's JSON (quotes and backslashes in
//! the piece are escaped again) stays within [`CHUNK_BYTES`]. Groups not completed within
//! [`REASSEMBLY_TIMEOUT_MS`] are dropped.

use std::collections::HashMap;

use thiserror::Error;

use crate::protocol::{Chunk, Envelope, PROTOCOL_VERSION};
use crate::text::decode_envelope;

/// Budget for one chunk's escaped piece (the envelope around it stays well under the
/// browser's 1 MB limit).
pub const CHUNK_BYTES: usize = 900 * 1024;
/// A chunk group must complete within this long.
pub const REASSEMBLY_TIMEOUT_MS: i64 = 10_000;
/// Most chunks in one group: enough for the largest IPC message (64 MiB) even when every
/// character doubles when escaped again.
pub const MAX_CHUNKS: u32 = 160;
/// Most bytes held for incomplete groups at once.
pub const MAX_PENDING_BYTES: usize = 160 * 1024 * 1024;
/// Most incomplete groups at once (normally one or two).
pub const MAX_GROUPS: usize = 64;
/// Longest group id (a UUID is 36 characters).
pub const MAX_GROUP_ID_LEN: usize = 64;
/// Longest message type that can be chunked.
pub const MAX_KIND_LEN: usize = 64;
/// Memory counted for each group besides its pieces (slots, id, bookkeeping).
const GROUP_OVERHEAD: usize = 1024;

/// Bytes a character takes inside a JSON string.
fn escaped_len(c: char) -> usize {
    match c {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        c if (c as u32) < 0x20 => 6,
        c => c.len_utf8(),
    }
}

/// Why a message can't be split.
#[derive(Debug, Error)]
pub enum SplitError {
    #[error("message can't be serialised: {0}")]
    Json(#[from] serde_json::Error),
    #[error("message type is over {MAX_KIND_LEN} bytes")]
    KindTooLong,
    #[error("message needs {0} chunks, over the {MAX_CHUNKS} the receiver accepts")]
    TooManyChunks(usize),
    #[error("frame limit {0} is too small for {CHUNK_BYTES}-byte chunks")]
    FrameTooSmall(usize),
}

/// `env` as frames that each fit in `max_frame` bytes of JSON (at least
/// [`CHUNK_BYTES`] + 64 KiB): itself if it fits, else chunks of its JSON. Pieces are cut at
/// character (code point) boundaries, so a chunk never holds half of a surrogate pair:
/// the extension must cut the same way (API Design §4.3).
pub fn split(env: &Envelope, max_frame: usize) -> Result<Vec<Envelope>, SplitError> {
    let json = serde_json::to_string(env)?;
    if json.len() <= max_frame {
        return Ok(vec![env.clone()]);
    }
    if max_frame < CHUNK_BYTES + 64 * 1024 {
        return Err(SplitError::FrameTooSmall(max_frame));
    }
    if env.kind.len() > MAX_KIND_LEN {
        return Err(SplitError::KindTooLong);
    }
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut budget = 0;
    for (i, c) in json.char_indices() {
        let len = escaped_len(c);
        if budget + len > CHUNK_BYTES {
            pieces.push(&json[start..i]);
            start = i;
            budget = 0;
        }
        budget += len;
    }
    pieces.push(&json[start..]);
    let total = u32::try_from(pieces.len()).unwrap_or(u32::MAX);
    if total > MAX_CHUNKS {
        return Err(SplitError::TooManyChunks(pieces.len()));
    }
    let group_id = uuid::Uuid::new_v4().to_string();
    Ok(pieces
        .into_iter()
        .zip(0..)
        .map(|(piece, seq)| Envelope {
            v: PROTOCOL_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            kind: env.kind.clone(),
            reply_to: None,
            chunk: Some(Chunk { seq, total, group_id: group_id.clone() }),
            payload: serde_json::Value::String(piece.to_owned()),
        })
        .collect())
}

/// A chunk that can't be used. The group it belongs to is dropped.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChunkError {
    #[error("chunk {seq} of {total} is out of range")]
    OutOfRange { seq: u32, total: u32 },
    #[error("chunk payload is not a string")]
    NotText,
    #[error("chunk group doesn't match its first chunk")]
    Inconsistent,
    #[error("too much incomplete chunked data")]
    TooLarge,
    #[error("reassembled message is invalid: {0}")]
    Invalid(String),
}

#[derive(Debug)]
struct Group {
    kind: String,
    parts: Vec<Option<String>>,
    received: u32,
    bytes: usize,
    started_at: i64,
}

/// Joins chunk groups back into envelopes.
#[derive(Debug, Default)]
pub struct Reassembler {
    groups: HashMap<String, Group>,
    pending_bytes: usize,
}

impl Reassembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops groups older than the timeout.
    pub fn expire(&mut self, now_ms: i64) {
        let pending = &mut self.pending_bytes;
        self.groups.retain(|_, g| {
            let keep = now_ms - g.started_at <= REASSEMBLY_TIMEOUT_MS;
            if !keep {
                *pending -= g.bytes;
            }
            keep
        });
    }

    fn drop_group(&mut self, id: &str) {
        if let Some(g) = self.groups.remove(id) {
            self.pending_bytes -= g.bytes;
        }
    }

    /// Takes any envelope. Unchunked ones come straight back; a chunk is held until its
    /// group is complete, then the original envelope is returned.
    pub fn push(&mut self, env: Envelope, now_ms: i64) -> Result<Option<Envelope>, ChunkError> {
        self.expire(now_ms);
        let Some(chunk) = env.chunk.clone() else { return Ok(Some(env)) };
        let Chunk { seq, total, group_id } = chunk;
        if total == 0 || total > MAX_CHUNKS || seq >= total {
            self.drop_group(&group_id);
            return Err(ChunkError::OutOfRange { seq, total });
        }
        let serde_json::Value::String(piece) = env.payload else {
            self.drop_group(&group_id);
            return Err(ChunkError::NotText);
        };
        if group_id.len() > MAX_GROUP_ID_LEN || env.kind.len() > MAX_KIND_LEN {
            return Err(ChunkError::TooLarge);
        }
        let new_group = !self.groups.contains_key(&group_id);
        let overhead = if new_group { GROUP_OVERHEAD + 8 * total as usize } else { 0 };
        if (new_group && self.groups.len() >= MAX_GROUPS)
            || self.pending_bytes + overhead + piece.len() > MAX_PENDING_BYTES
        {
            self.drop_group(&group_id);
            return Err(ChunkError::TooLarge);
        }
        self.pending_bytes += overhead;
        let group = self.groups.entry(group_id.clone()).or_insert_with(|| Group {
            kind: env.kind.clone(),
            parts: vec![None; total as usize],
            received: 0,
            bytes: overhead,
            started_at: now_ms,
        });
        if group.parts.len() != total as usize || group.kind != env.kind {
            self.drop_group(&group_id);
            return Err(ChunkError::Inconsistent);
        }
        let slot = &mut group.parts[seq as usize];
        if slot.is_none() {
            group.received += 1;
            group.bytes += piece.len();
            self.pending_bytes += piece.len();
            *slot = Some(piece);
        }
        if group.received < total {
            return Ok(None);
        }
        let group = self.groups.remove(&group_id).ok_or(ChunkError::Inconsistent)?;
        self.pending_bytes -= group.bytes;
        let json: String = group.parts.into_iter().flatten().collect();
        let original =
            decode_envelope(json.as_bytes()).map_err(|e| ChunkError::Invalid(e.to_string()))?;
        if original.chunk.is_some() || original.kind != group.kind {
            return Err(ChunkError::Invalid("nested or mismatched chunk".into()));
        }
        Ok(Some(original))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::MAX_TO_BROWSER;
    use serde_json::json;

    fn big(kind: &str, text: &str, repeat: usize) -> Envelope {
        Envelope::new(kind, &json!({ "titles": vec![text; repeat] })).unwrap()
    }

    #[test]
    fn small_messages_are_not_split() {
        let env = big("snapshot", "a", 3);
        assert_eq!(split(&env, MAX_TO_BROWSER).unwrap(), vec![env]);
    }

    #[test]
    fn splits_within_the_browser_limit_and_reassembles_in_any_order() {
        // Quotes, backslashes, control characters and emoji all grow when escaped again.
        let env = big("snapshot", "\"\\\u{1}😀é tab title", 200_000);
        let chunks = split(&env, MAX_TO_BROWSER).unwrap();
        assert!(chunks.len() > 3);
        for c in &chunks {
            assert!(serde_json::to_vec(c).unwrap().len() <= MAX_TO_BROWSER);
        }
        let mut r = Reassembler::new();
        let mut out = None;
        for c in chunks.into_iter().rev() {
            assert_eq!(out, None);
            out = r.push(c, 0).unwrap();
        }
        assert_eq!(out, Some(env));
        assert_eq!(r.pending_bytes, 0);
    }

    #[test]
    fn duplicate_chunks_are_ignored_and_stale_groups_expire() {
        let env = big("snapshot", "x", 400_000);
        let chunks = split(&env, MAX_TO_BROWSER).unwrap();
        let mut r = Reassembler::new();
        r.push(chunks[0].clone(), 0).unwrap();
        r.push(chunks[0].clone(), 0).unwrap();
        assert_eq!(r.groups.values().next().unwrap().received, 1);
        // The rest arrive too late: the group was dropped, so they never complete.
        for c in &chunks[1..] {
            assert_eq!(r.push(c.clone(), REASSEMBLY_TIMEOUT_MS + 1).unwrap(), None);
        }
        assert_eq!(r.groups.len(), 1, "a new partial group from the late chunks");
        r.expire(3 * REASSEMBLY_TIMEOUT_MS);
        assert_eq!((r.groups.len(), r.pending_bytes), (0, 0));
    }

    #[test]
    fn refuses_to_split_into_more_chunks_than_the_receiver_takes() {
        // The largest IPC message (64 MiB of JSON: 32 MiB of quotes, each escaped as `\"`)
        // doubles again inside chunks: ~146 chunks, within the limit.
        let quotes = Envelope::new("snapshot", &"\"".repeat(32 * 1024 * 1024 - 64)).unwrap();
        let chunks = split(&quotes, MAX_TO_BROWSER).unwrap();
        assert!(chunks.len() > 100 && chunks.len() <= MAX_CHUNKS as usize, "{}", chunks.len());
        let huge = Envelope::new("snapshot", &"\"".repeat(80 * 1024 * 1024)).unwrap();
        assert!(matches!(split(&huge, MAX_TO_BROWSER), Err(SplitError::TooManyChunks(_))));
        let long_kind = Envelope::new(&"k".repeat(600_000), &"x".repeat(2_000_000)).unwrap();
        assert!(matches!(split(&long_kind, MAX_TO_BROWSER), Err(SplitError::KindTooLong)));
        assert!(matches!(split(&huge, 1000), Err(SplitError::FrameTooSmall(1000))));
    }

    #[test]
    fn caps_groups_and_their_bookkeeping() {
        let chunk = |group: String| Envelope {
            v: 1,
            id: "i".into(),
            kind: "snapshot".into(),
            reply_to: None,
            chunk: Some(Chunk { seq: 0, total: 2, group_id: group }),
            payload: json!(""),
        };
        let mut r = Reassembler::new();
        for i in 0..MAX_GROUPS {
            assert_eq!(r.push(chunk(format!("g{i}")), 0), Ok(None));
        }
        assert_eq!(r.push(chunk("one-too-many".into()), 0), Err(ChunkError::TooLarge));
        assert_eq!(r.push(chunk("g".repeat(MAX_GROUP_ID_LEN + 1)), 0), Err(ChunkError::TooLarge));
        assert!(r.pending_bytes >= MAX_GROUPS * GROUP_OVERHEAD, "empty pieces still count");
        r.expire(REASSEMBLY_TIMEOUT_MS + 1);
        assert_eq!((r.groups.len(), r.pending_bytes), (0, 0));
    }

    #[test]
    fn rejects_hostile_chunks() {
        let chunk = |seq, total, payload: serde_json::Value| Envelope {
            v: 1,
            id: "i".into(),
            kind: "snapshot".into(),
            reply_to: None,
            chunk: Some(Chunk { seq, total, group_id: "g".into() }),
            payload,
        };
        let mut r = Reassembler::new();
        assert!(matches!(r.push(chunk(5, 5, json!("x")), 0), Err(ChunkError::OutOfRange { .. })));
        assert!(matches!(r.push(chunk(0, 0, json!("x")), 0), Err(ChunkError::OutOfRange { .. })));
        assert!(matches!(
            r.push(chunk(0, MAX_CHUNKS + 1, json!("x")), 0),
            Err(ChunkError::OutOfRange { .. })
        ));
        assert_eq!(r.push(chunk(0, 1, json!(1)), 0), Err(ChunkError::NotText));
        r.push(chunk(0, 2, json!("{")), 0).unwrap();
        assert_eq!(r.push(chunk(1, 3, json!("}")), 0), Err(ChunkError::Inconsistent));
        assert!(matches!(r.push(chunk(0, 1, json!("not json")), 0), Err(ChunkError::Invalid(_))));
        // A chunk can't smuggle in another chunk.
        let inner = serde_json::to_string(&chunk(0, 1, json!("x"))).unwrap();
        assert!(matches!(r.push(chunk(0, 1, json!(inner)), 0), Err(ChunkError::Invalid(_))));
        assert_eq!(r.pending_bytes, 0);
    }
}
