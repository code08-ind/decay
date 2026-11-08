//! Length-prefixed frames: a `u32` little-endian length, then that many bytes of JSON.
//! The same framing serves native messaging (stdio, where Chrome uses native byte order:
//! little-endian on every platform Decay supports) and the host ⇄ app IPC (API Design §5).

use std::io::{self, Read, Write};

use crate::protocol::Envelope;
use crate::text::decode_envelope;

#[cfg(target_endian = "big")]
compile_error!("native messaging framing assumes a little-endian platform");

/// Largest message the browser accepts from a native host.
pub const MAX_TO_BROWSER: usize = 1024 * 1024;
/// Largest message accepted from the browser (Chrome's own cap is 64 MiB).
pub const MAX_FROM_BROWSER: usize = 64 * 1024 * 1024;
/// Largest IPC frame (a browser message plus the host's connection metadata).
pub const MAX_IPC_FRAME: usize = MAX_FROM_BROWSER + 64 * 1024;

/// Reads one frame. `Ok(None)` on a clean end of stream before a frame starts; an error for
/// a frame over `max` bytes (nothing more is read: the stream can't be trusted after it)
/// or a stream that ends mid-frame.
pub fn read_frame(r: &mut impl Read, max: usize) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match r.read(&mut len[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame of {len} bytes is over the {max}-byte limit"),
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(Some(buf))
}

/// Writes one frame (length and body in a single write, then flushes).
pub fn write_frame(w: &mut impl Write, body: &[u8]) -> io::Result<()> {
    let len = u32::try_from(body.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame over 4 GiB"))?;
    let mut buf = Vec::with_capacity(body.len() + 4);
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(body);
    w.write_all(&buf)?;
    w.flush()
}

/// Reads one envelope frame; invalid JSON is an `InvalidData` error.
pub fn read_envelope(r: &mut impl Read, max: usize) -> io::Result<Option<Envelope>> {
    let Some(bytes) = read_frame(r, max)? else { return Ok(None) };
    decode_envelope(&bytes).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Writes one envelope frame.
pub fn write_envelope(w: &mut impl Write, env: &Envelope) -> io::Result<()> {
    let body =
        serde_json::to_vec(env).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write_frame(w, &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn frames_round_trip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"{\"a\":1}").unwrap();
        write_frame(&mut buf, b"").unwrap();
        assert_eq!(&buf[..4], &7u32.to_le_bytes());
        let mut r = Cursor::new(buf);
        assert_eq!(read_frame(&mut r, 100).unwrap().unwrap(), b"{\"a\":1}");
        assert_eq!(read_frame(&mut r, 100).unwrap().unwrap(), b"");
        assert_eq!(read_frame(&mut r, 100).unwrap(), None);
    }

    #[test]
    fn rejects_oversized_and_truncated_frames() {
        let mut buf = Vec::new();
        write_frame(&mut buf, &[b'x'; 50]).unwrap();
        let err = read_frame(&mut Cursor::new(buf.clone()), 49).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        buf.truncate(20);
        let err = read_frame(&mut Cursor::new(buf), 100).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        let err = read_frame(&mut Cursor::new(vec![1u8, 0]), 100).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        // A 4 GiB length is refused without allocating.
        let err = read_frame(&mut Cursor::new(u32::MAX.to_le_bytes().to_vec()), 100).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
