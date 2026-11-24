//! Decay Pro licenses (P4.2, PRD F27, API Design §7.2): checked offline, no account, no
//! activation server, no device limit.
//!
//! - **Key:** `DECAY-` + base32 (no padding) of `payload ‖ signature`, where `payload` is
//!   the compact JSON `{ v: 1, email, tier: "pro", issuedAt, licenseId }` and `signature`
//!   the 64-byte Ed25519 signature of `payload`. It's shown in groups of five; dashes,
//!   spaces, line breaks and letter case are ignored when it's read back.
//! - **Verified** with [`PUBLIC_KEY`] (the private key never leaves the issuer: P4.3's
//!   purchase webhook, or `decay-license-tool issue`).
//! - **Trial:** 14 days of Pro, started by the user once per installation.
//! - **Tier:** a valid key → Pro; else a running trial → Trial; else Free.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Every key starts with this.
pub const KEY_PREFIX: &str = "DECAY-";
/// The Pro trial's length.
pub const TRIAL_DAYS: i64 = 14;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Keys longer than this aren't read (the deep link allows 512 characters).
pub const MAX_KEY_LEN: usize = 512;
const SIG_LEN: usize = 64;

/// The public half of the license signing key (generated 24 Nov 2026; the private half is
/// kept outside the repository: Docs/Release.md "Licenses").
pub const PUBLIC_KEY: [u8; 32] = include!("public_key.in");

/// What a license key says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct License {
    pub v: u32,
    pub email: String,
    pub tier: String,
    /// Epoch ms.
    pub issued_at: i64,
    pub license_id: String,
}

/// Why a key isn't accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LicenseError {
    /// Not a Decay key at all (wrong prefix, not base32, too short or too long).
    Format,
    /// Well formed, but not signed by Decay (or changed since).
    Signature,
    /// Signed, but for a version or tier this app doesn't know.
    Unsupported,
}

impl std::fmt::Display for LicenseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Format => "that isn't a Decay license key",
            Self::Signature => "this license key isn't valid",
            Self::Unsupported => "this license key is for a newer version of Decay",
        })
    }
}

impl std::error::Error for LicenseError {}

/// Checks a key against [`PUBLIC_KEY`].
pub fn verify(key: &str) -> Result<License, LicenseError> {
    verify_with(key, &PUBLIC_KEY)
}

/// Checks a key against `public_key`.
pub fn verify_with(key: &str, public_key: &[u8; 32]) -> Result<License, LicenseError> {
    if key.len() > MAX_KEY_LEN {
        return Err(LicenseError::Format);
    }
    let compact: String = key.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    let compact = compact.to_ascii_uppercase();
    let body = compact.strip_prefix("DECAY").ok_or(LicenseError::Format)?;
    let bytes =
        data_encoding::BASE32_NOPAD.decode(body.as_bytes()).map_err(|_| LicenseError::Format)?;
    if bytes.len() <= SIG_LEN {
        return Err(LicenseError::Format);
    }
    let (payload, sig) = bytes.split_at(bytes.len() - SIG_LEN);
    let sig = Signature::from_slice(sig).map_err(|_| LicenseError::Format)?;
    let verifying = VerifyingKey::from_bytes(public_key).map_err(|_| LicenseError::Signature)?;
    verifying.verify_strict(payload, &sig).map_err(|_| LicenseError::Signature)?;
    let license: License =
        serde_json::from_slice(payload).map_err(|_| LicenseError::Unsupported)?;
    if license.v != 1 || license.tier != "pro" {
        return Err(LicenseError::Unsupported);
    }
    Ok(license)
}

/// Signs a license with the private key's 32 secret bytes (the issuer only).
///
/// # Panics
/// Never: a `License` (strings and numbers) always serialises.
pub fn issue(license: &License, secret: &[u8; 32]) -> String {
    let payload = serde_json::to_vec(license).expect("a license always serialises");
    let sig = SigningKey::from_bytes(secret).sign(&payload);
    let mut bytes = payload;
    bytes.extend_from_slice(&sig.to_bytes());
    let body = data_encoding::BASE32_NOPAD.encode(&bytes);
    let groups: Vec<&str> =
        body.as_bytes().chunks(5).map(|c| std::str::from_utf8(c).unwrap_or("")).collect();
    format!("{KEY_PREFIX}{}", groups.join("-"))
}

/// The public key for a private key's secret bytes.
pub fn public_key_of(secret: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(secret).verifying_key().to_bytes()
}

/// Which features are on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Free,
    Trial,
    Pro,
}

impl Tier {
    /// Pro features are on (Pro, or a running trial).
    pub fn is_pro(self) -> bool {
        self != Self::Free
    }
}

/// What the app stores about the license (the `license` row).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stored {
    pub key: Option<String>,
    pub trial_started_at: Option<i64>,
}

/// The license as the app shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub tier: Tier,
    /// Whole days left in a running trial (the current day counts).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial_days_left: Option<u32>,
    /// When the trial ends / ended (epoch ms), once one was started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial_ends_at: Option<i64>,
    /// A trial can still be started (never started on this installation).
    pub trial_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// The tier at `now`. A stored key that no longer verifies (another build's key, an edited
/// database) counts as none.
pub fn status(stored: &Stored, now: i64) -> Status {
    status_with(stored, now, &PUBLIC_KEY)
}

/// [`status`] against `public_key`.
pub fn status_with(stored: &Stored, now: i64, public_key: &[u8; 32]) -> Status {
    let license = stored.key.as_deref().and_then(|k| verify_with(k, public_key).ok());
    let trial_ends_at = stored.trial_started_at.map(|t| t.saturating_add(TRIAL_DAYS * DAY_MS));
    let trial_days_left = trial_ends_at.filter(|end| now < *end).map(|end| {
        // A clock set back before the start shows the whole trial, never more.
        let left = (end - now).min(TRIAL_DAYS * DAY_MS);
        u32::try_from((left + DAY_MS - 1) / DAY_MS).unwrap_or(0)
    });
    let tier = match (&license, trial_days_left) {
        (Some(_), _) => Tier::Pro,
        (None, Some(_)) => Tier::Trial,
        (None, None) => Tier::Free,
    };
    Status {
        tier,
        trial_days_left: trial_days_left.filter(|_| tier == Tier::Trial),
        trial_ends_at,
        trial_available: stored.trial_started_at.is_none(),
        email: license.map(|l| l.email),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: [u8; 32] = [7; 32];
    const NOW: i64 = 1_800_000_000_000;

    fn license() -> License {
        License {
            v: 1,
            email: "me@example.com".into(),
            tier: "pro".into(),
            issued_at: NOW,
            license_id: "lic_1".into(),
        }
    }

    fn public() -> [u8; 32] {
        public_key_of(&SECRET)
    }

    #[test]
    fn an_issued_key_verifies_however_it_is_typed() {
        let key = issue(&license(), &SECRET);
        assert!(key.starts_with("DECAY-"));
        assert!(key.len() <= MAX_KEY_LEN, "{}", key.len());
        assert_eq!(verify_with(&key, &public()), Ok(license()));
        let messy = format!("  {}\n", key.to_lowercase().replace('-', " "));
        assert_eq!(verify_with(&messy, &public()), Ok(license()));
    }

    #[test]
    fn keys_from_anyone_else_or_changed_are_refused() {
        let key = issue(&license(), &SECRET);
        assert_eq!(verify_with(&key, &public_key_of(&[8; 32])), Err(LicenseError::Signature));
        // One payload character changed.
        let mut tampered: Vec<char> = key.chars().collect();
        let i = 12;
        tampered[i] = if tampered[i] == 'A' { 'B' } else { 'A' };
        let tampered: String = tampered.into_iter().collect();
        assert!(verify_with(&tampered, &public()).is_err());
        for bad in ["", "DECAY-", "DECAY-!!!!", "HELLO-ABCDE", "DECAY-ABCDEFGH"] {
            assert_eq!(verify_with(bad, &public()), Err(LicenseError::Format), "{bad:?}");
        }
        let long = format!("DECAY-{}", "A".repeat(600));
        assert_eq!(verify_with(&long, &public()), Err(LicenseError::Format));
    }

    #[test]
    fn unknown_versions_and_tiers_are_refused() {
        let v2 = License { v: 2, ..license() };
        assert_eq!(verify_with(&issue(&v2, &SECRET), &public()), Err(LicenseError::Unsupported));
        let team = License { tier: "team".into(), ..license() };
        assert_eq!(verify_with(&issue(&team, &SECRET), &public()), Err(LicenseError::Unsupported));
    }

    #[test]
    fn the_tier_follows_the_key_and_the_trial() {
        let st = |key: Option<String>, trial: Option<i64>, now| {
            status_with(&Stored { key, trial_started_at: trial }, now, &public())
        };
        let fresh = st(None, None, NOW);
        assert_eq!(
            (fresh.tier, fresh.trial_available, fresh.trial_days_left),
            (Tier::Free, true, None)
        );
        let day1 = st(None, Some(NOW), NOW);
        assert_eq!(
            (day1.tier, day1.trial_days_left, day1.trial_available),
            (Tier::Trial, Some(14), false)
        );
        assert!(day1.tier.is_pro());
        let last = st(None, Some(NOW), NOW + 13 * DAY_MS + 1);
        assert_eq!(last.trial_days_left, Some(1));
        let over = st(None, Some(NOW), NOW + 14 * DAY_MS);
        assert_eq!(
            (over.tier, over.trial_days_left, over.trial_available),
            (Tier::Free, None, false)
        );
        assert_eq!(over.trial_ends_at, Some(NOW + 14 * DAY_MS));
        let rolled_back = st(None, Some(NOW), NOW - 30 * DAY_MS);
        assert_eq!(rolled_back.trial_days_left, Some(14), "never more than the whole trial");
        let key = issue(&license(), &SECRET);
        let pro = st(Some(key.clone()), Some(NOW), NOW + 100 * DAY_MS);
        assert_eq!((pro.tier, pro.email.as_deref()), (Tier::Pro, Some("me@example.com")));
        let pro_in_trial = st(Some(key), Some(NOW), NOW);
        assert_eq!((pro_in_trial.tier, pro_in_trial.trial_days_left), (Tier::Pro, None));
        let broken = st(Some("DECAY-XXXXX".into()), None, NOW);
        assert_eq!(broken.tier, Tier::Free);
        assert!(!Tier::Free.is_pro());
    }

    #[test]
    fn the_status_serialises_for_the_app() {
        let s = status_with(&Stored { key: None, trial_started_at: Some(NOW) }, NOW, &public());
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["tier"], "trial");
        assert_eq!(json["trialDaysLeft"], 14);
        assert_eq!(json["trialAvailable"], false);
        assert!(json.get("email").is_none());
    }

    #[test]
    fn the_embedded_public_key_is_a_real_key() {
        assert!(VerifyingKey::from_bytes(&PUBLIC_KEY).is_ok());
    }
}
