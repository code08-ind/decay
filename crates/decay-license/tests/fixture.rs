//! The key format is shared with the purchase webhook (services/license-webhook, P4.3),
//! which issues keys in TypeScript: both must produce byte-identical keys. This fixture
//! pins Rust's output; the webhook's tests check theirs against the same file.
//! `UPDATE_FIXTURES=1 cargo test -p decay-license` rewrites it (only for a deliberate format change).

use decay_license::{License, issue, public_key_of, verify_with};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/issued.json");
/// A test-only secret (never a real signing key).
const SEED: [u8; 32] = [7; 32];

fn license() -> License {
    License {
        v: 1,
        email: "jörg+pro@example.com".into(),
        tier: "pro".into(),
        issued_at: 1_795_500_000_000,
        license_id: "ls-123456".into(),
    }
}

#[test]
fn issued_keys_match_the_shared_fixture() {
    let key = issue(&license(), &SEED);
    assert_eq!(verify_with(&key, &public_key_of(&SEED)), Ok(license()));
    let seed_hex = "07".repeat(32);
    let expected = serde_json::json!({
        "seedHex": seed_hex,
        "license": license(),
        "key": key,
    });
    let text = format!("{}\n", serde_json::to_string_pretty(&expected).unwrap());
    if std::env::var_os("UPDATE_FIXTURES").is_some() {
        std::fs::write(FIXTURE, &text).unwrap();
    }
    let stored =
        std::fs::read_to_string(FIXTURE).expect("the fixture (UPDATE_FIXTURES=1 writes it)");
    assert_eq!(stored.replace("\r\n", "\n"), text, "the key format changed");
}
