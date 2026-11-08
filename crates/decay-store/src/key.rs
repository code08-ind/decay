//! The database key: 256 random bits, kept by the app in the Keychain / Credential Manager
//! (DB Design §1). decay-store only uses it; storing it is the app's job.

use std::fmt;

/// A 256-bit `SQLCipher` key, used as a raw key (no passphrase derivation). Zeroed on drop
/// and never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct DbKey([u8; 32]);

impl DbKey {
    /// A new random key from the OS random source.
    pub fn generate() -> Result<Self, getrandom::Error> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes)?;
        Ok(Self(bytes))
    }

    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Parses the 64-hex-digit form (as stored in the Keychain).
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.as_bytes();
        if hex.len() != 64 {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (i, pair) in hex.as_chunks::<2>().0.iter().enumerate() {
            let s = std::str::from_utf8(pair).ok()?;
            bytes[i] = u8::from_str_radix(s, 16).ok()?;
        }
        Some(Self(bytes))
    }

    /// The 64-hex-digit form.
    pub fn to_hex(&self) -> String {
        use std::fmt::Write as _;
        self.0.iter().fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }

    /// `PRAGMA key` statement for this key.
    pub(crate) fn pragma(&self) -> String {
        format!("PRAGMA key = \"x'{}'\";", self.to_hex())
    }
}

impl fmt::Debug for DbKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DbKey(…)")
    }
}

impl Drop for DbKey {
    fn drop(&mut self) {
        // Best effort (volatile writes need `unsafe`, which the workspace forbids): plain
        // writes plus a compiler fence so they aren't optimised away.
        self.0.fill(0);
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_debug_hides_the_key() {
        let key = DbKey::generate().unwrap();
        assert_eq!(DbKey::from_hex(&key.to_hex()), Some(key.clone()));
        assert_eq!(format!("{key:?}"), "DbKey(…)");
        assert_eq!(DbKey::from_hex("abc"), None);
        assert_eq!(DbKey::from_hex(&"zz".repeat(32)), None);
        assert_ne!(DbKey::generate().unwrap(), key);
    }
}
