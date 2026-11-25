import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import { licenseMail } from '../src/email';
import { type Env, handle } from '../src/index';
import { base32, hexSeed, issue, type License } from '../src/license';
import { fetchCustomer, MAX_AGE_S, parseTransaction, verifySignature } from '../src/paddle';

const fixture = JSON.parse(
  readFileSync(join(__dirname, '../../../crates/decay-license/tests/fixtures/issued.json'), 'utf8'),
) as { seedHex: string; key: string; license: License };

const SECRET = 'pdl_ntfset_test';
const NOW_S = 1_795_500_000;
const env: Env = {
  PADDLE_WEBHOOK_SECRET: SECRET,
  PADDLE_API_KEY: 'pdl_apikey_test',
  LICENSE_SIGNING_KEY: fixture.seedHex,
  RESEND_API_KEY: 're_test',
  EMAIL_FROM: 'Decay <licenses@example.com>',
  PADDLE_PRICE_IDS: 'pri_decaypro',
  PADDLE_ENV: 'sandbox',
};

const txn = (over: Record<string, unknown> = {}, event = 'transaction.completed') => ({
  event_id: 'evt_1',
  event_type: event,
  occurred_at: '2026-11-24T10:00:01Z',
  data: {
    id: 'txn_01abc',
    status: 'completed',
    customer_id: 'ctm_01xyz',
    billed_at: '2026-11-24T10:00:00Z',
    created_at: '2026-11-24T09:59:00Z',
    items: [{ price: { id: 'pri_decaypro', product_id: 'pro_decay' }, quantity: 1 }],
    ...over,
  },
});

async function sign(body: string, ts = NOW_S, secret = SECRET): Promise<string> {
  const key = await crypto.subtle.importKey(
    'raw',
    new TextEncoder().encode(secret),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign'],
  );
  const mac = new Uint8Array(
    await crypto.subtle.sign('HMAC', key, new TextEncoder().encode(`${ts}:${body}`)),
  );
  return `ts=${ts};h1=${[...mac].map((b) => b.toString(16).padStart(2, '0')).join('')}`;
}

/** Paddle's customers API and Resend, faked. */
function services(opts: { customer?: unknown; resend?: number } = {}) {
  return vi.fn(async (url: string | URL | Request) => {
    const u = String(url);
    if (u.includes('/customers/')) {
      return new Response(
        JSON.stringify(opts.customer ?? { data: { email: 'buyer@example.com', name: 'Ada' } }),
        { status: 200 },
      );
    }
    return new Response('{}', { status: opts.resend ?? 200 });
  });
}

async function post(
  body: unknown,
  opts: { signature?: string; path?: string; fetcher?: ReturnType<typeof services> } = {},
) {
  const text = JSON.stringify(body);
  const req = new Request(`https://hooks.example${opts.path ?? '/webhooks/purchase'}`, {
    method: 'POST',
    body: text,
    headers: { 'Paddle-Signature': opts.signature ?? (await sign(text)) },
  });
  const fetcher = opts.fetcher ?? services();
  const res = await handle(req, env, fetcher as unknown as typeof fetch, NOW_S);
  return { status: res.status, text: await res.text(), fetcher };
}

describe('license keys', () => {
  it('match the Rust issuer byte for byte (shared fixture)', async () => {
    const l = fixture.license;
    const key = await issue(
      { v: 1, email: l.email, tier: 'pro', issuedAt: l.issuedAt, licenseId: l.licenseId },
      hexSeed(fixture.seedHex),
    );
    expect(key).toBe(fixture.key);
  });

  it('base32 follows RFC 4648 without padding', () => {
    const b = (s: string) => base32(new TextEncoder().encode(s));
    expect(b('')).toBe('');
    expect(b('f')).toBe('MY');
    expect(b('fooba')).toBe('MZXW6YTB');
    expect(b('foobar')).toBe('MZXW6YTBOI');
  });

  it('refuses a signing key that is not 64 hex characters', () => {
    expect(() => hexSeed('abc')).toThrow();
    expect(() => hexSeed('z'.repeat(64))).toThrow();
  });
});

describe('Paddle signatures', () => {
  it('accept the signed body within five minutes, any of several secrets', async () => {
    const body = '{"a":1}';
    expect(await verifySignature(SECRET, body, await sign(body), NOW_S)).toBe(true);
    expect(await verifySignature(SECRET, body, await sign(body), NOW_S + MAX_AGE_S)).toBe(true);
    // A secret being rotated: one of the h1 values matches.
    const good = (await sign(body)).split(';h1=')[1];
    const rotating = `ts=${NOW_S};h1=${'0'.repeat(64)};h1=${good}`;
    expect(await verifySignature(SECRET, body, rotating, NOW_S)).toBe(true);
  });

  it('refuse a changed body, another secret, a stale or missing timestamp', async () => {
    const body = '{"a":1}';
    expect(await verifySignature(SECRET, '{"a":2}', await sign(body), NOW_S)).toBe(false);
    expect(await verifySignature(SECRET, body, await sign(body, NOW_S, 'other'), NOW_S)).toBe(
      false,
    );
    expect(await verifySignature(SECRET, body, await sign(body), NOW_S + MAX_AGE_S + 1)).toBe(
      false,
    );
    expect(await verifySignature(SECRET, body, 'h1=abc', NOW_S)).toBe(false);
    expect(await verifySignature(SECRET, body, null, NOW_S)).toBe(false);
    expect(await verifySignature('', body, await sign(body), NOW_S)).toBe(false);
  });
});

describe('the webhook', () => {
  it('emails a key for a completed Decay Pro purchase, the same key on a retry', async () => {
    const first = await post(txn());
    expect([first.status, first.text]).toEqual([200, 'sent']);
    const calls = first.fetcher.mock.calls as unknown as [string, RequestInit][];
    expect(calls[0]?.[0]).toBe('https://sandbox-api.paddle.com/customers/ctm_01xyz');
    expect((calls[0]?.[1]?.headers as Record<string, string> | undefined)?.Authorization).toBe(
      'Bearer pdl_apikey_test',
    );
    const [url, init] = calls[1] as [string, RequestInit];
    expect(url).toBe('https://api.resend.com/emails');
    expect((init.headers as Record<string, string>)['Idempotency-Key']).toBe('decay-txn_01abc');
    const mail = JSON.parse(String(init.body));
    expect(mail.to).toBe('buyer@example.com');
    const key = /DECAY-[A-Z2-7-]+/.exec(mail.text)?.[0];
    expect(key).toBeTruthy();
    const again = await post(txn());
    const mail2 = JSON.parse(
      String((again.fetcher.mock.calls[1] as unknown as [string, RequestInit])[1].body),
    );
    expect(/DECAY-[A-Z2-7-]+/.exec(mail2.text)?.[0]).toBe(key);
  });

  it('refuses a bad signature and answers only its own path', async () => {
    expect((await post(txn(), { signature: 'ts=1;h1=00' })).status).toBe(401);
    expect((await post(txn(), { path: '/other' })).status).toBe(404);
  });

  it('ignores what is not a completed Decay Pro purchase (200, nothing sent)', async () => {
    for (const body of [
      txn({}, 'transaction.created'),
      txn({ status: 'paid' }),
      txn({ items: [{ price: { id: 'pri_other' } }] }),
      txn({ id: 'nope' }),
      txn({ customer_id: '' }),
    ]) {
      const r = await post(body);
      expect(r.status).toBe(200);
      expect(r.text.startsWith('ignored')).toBe(true);
      expect(r.fetcher).not.toHaveBeenCalled();
    }
  });

  it('fails (so Paddle retries) when the customer or the email fails', async () => {
    const err = vi.spyOn(console, 'error').mockImplementation(() => {});
    expect((await post(txn(), { fetcher: services({ resend: 503 }) })).status).toBe(500);
    expect(
      (await post(txn(), { fetcher: services({ customer: { data: { email: 'x' } } }) })).status,
    ).toBe(500);
    expect(String(err.mock.calls[0]?.[0])).not.toContain('buyer@example.com');
    err.mockRestore();
  });

  it('reads the customer from the live API outside the sandbox', async () => {
    const f = services();
    await fetchCustomer('k', 'ctm_1', false, f as unknown as typeof fetch);
    expect(String(f.mock.calls[0]?.[0])).toBe('https://api.paddle.com/customers/ctm_1');
  });

  it('parses transactions defensively', () => {
    expect(parseTransaction(null, [])).toEqual({ ignore: 'event undefined' });
    expect('transaction' in parseTransaction(txn({ items: [] }), [])).toBe(true);
    expect(parseTransaction(txn({ billed_at: null, created_at: 'x' }), [])).toEqual({
      ignore: 'malformed transaction',
    });
  });
});

describe('the email', () => {
  it('carries the key, an activation link and escapes the name', () => {
    const m = licenseMail('Decay <a@b.c>', 'x@y.z', '<b>Eve</b>', 'DECAY-ABCDE');
    expect(m.text).toContain('DECAY-ABCDE');
    expect(m.text).toContain('https://decay.aryangarg.com/activate#key=DECAY-ABCDE');
    expect(m.html).toContain('href="https://decay.aryangarg.com/activate#key=DECAY-ABCDE"');
    expect(m.html).toContain('https://decay.aryangarg.com/icon.png');
    expect(m.reply_to).toBe('support.decay@aryangarg.com');
    expect(m.html).toContain('&lt;b&gt;Eve&lt;/b&gt;');
    expect(m.html).not.toContain('<b>Eve</b>');
  });
});
