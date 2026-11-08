//! The link between the browser extension and the Decay app (Software Design §4.6–4.8,
//! API Design §4–5):
//!
//! ```text
//! extension ──native messaging (stdio)──▶ decay-host ──local socket / named pipe──▶ app
//! ```
//!
//! - [`protocol`]: the envelope and payload types.
//! - [`frame`]: `u32` length-prefixed JSON frames, with size limits.
//! - [`chunk`]: splitting messages over the browser's 1 MB limit, and joining them.
//! - [`text`]: decoding browser JSON safely (lone surrogates, invalid UTF-8).
//! - [`ipc`]: the host ⇄ app connection, private to the current user.

pub mod chunk;
pub mod frame;
pub mod ipc;
pub mod protocol;
pub mod text;

pub use protocol::{Envelope, ErrorCode, HOST_NAME, PROTOCOL_VERSION, kinds};
