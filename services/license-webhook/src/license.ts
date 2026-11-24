// Decay Pro license keys, issued exactly as decay-license (Rust) does: `DECAY-` + base32
// (RFC 4648, no padding) of `payload ‖ Ed25519 signature`, in groups of five. The payload
// is compact JSON with the fields in this order. The shared fixture
// (crates/decay-license/tests/fixtures/issued.json) keeps both byte-identical.

export interface License {
  v: 1;
  email: string;
  tier: 'pro';
  issuedAt: number;
  licenseId: string;
}

const ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';

export function base32(bytes: Uint8Array): string {
  let out = '';
  let bits = 0;
  let value = 0;
  for (const b of bytes) {
    value = (value << 8) | b;
    bits += 8;
    while (bits >= 5) {
      out += ALPHABET[(value >>> (bits - 5)) & 31];
      bits -= 5;
    }
  }
  if (bits > 0) out += ALPHABET[(value << (5 - bits)) & 31];
  return out;
}

/** 64 hex characters → 32 bytes; throws on anything else. */
export function hexSeed(hex: string): Uint8Array {
  const clean = hex.trim();
  if (!/^[0-9a-fA-F]{64}$/.test(clean))
    throw new Error('the signing key must be 64 hex characters');
  const out = new Uint8Array(32);
  for (let i = 0; i < 32; i++) out[i] = Number.parseInt(clean.slice(i * 2, i * 2 + 2), 16);
  return out;
}

// PKCS#8 for an Ed25519 private key: this prefix, then the 32-byte seed.
const PKCS8_PREFIX = [
  0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

async function signingKey(seed: Uint8Array): Promise<CryptoKey> {
  const pkcs8 = new Uint8Array(48);
  pkcs8.set(PKCS8_PREFIX);
  pkcs8.set(seed, 16);
  return crypto.subtle.importKey('pkcs8', pkcs8, { name: 'Ed25519' }, false, ['sign']);
}

/** The key for `license`, signed with the private key's 32-byte seed. */
export async function issue(license: License, seed: Uint8Array): Promise<string> {
  const payload = new TextEncoder().encode(
    JSON.stringify({
      v: license.v,
      email: license.email,
      tier: license.tier,
      issuedAt: license.issuedAt,
      licenseId: license.licenseId,
    }),
  );
  const sig = new Uint8Array(await crypto.subtle.sign('Ed25519', await signingKey(seed), payload));
  const bytes = new Uint8Array(payload.length + sig.length);
  bytes.set(payload);
  bytes.set(sig, payload.length);
  const groups = base32(bytes).match(/.{1,5}/g) ?? [];
  return `DECAY-${groups.join('-')}`;
}
