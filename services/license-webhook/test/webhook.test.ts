import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import { licenseMail } from '../src/email';
import { type Env, handle } from '../src/index';
import { parseOrder } from '../src/lemonsqueezy';
import { base32, hexSeed, issue, type License } from '../src/license';

const fixture = JSON.parse(
  readFileSync(join(__dirname, '../../../crates/decay-license/tests/fixtures/issued.json'), 'utf8'),
) as { seedHex: string; key: string; license: License };

const SECRET = 'whsec_test';
const env: Env = {
  LS_SIGNING_SECRET: SECRET,
  LICENSE_SIGNING_KEY: fixture.seedHex,
  RESEND_API_KEY: 're_test',
  EMAIL_FROM: 'Decay <licenses@example.com>',
  LS_VARIANT_IDS: '42',
  ALLOW_TEST_MODE: '0',
};

const order = (over: Record<string, unknown> = {}) => ({
  meta: { event_name: 'order_created' },
  data: {
    type: 'orders',
    id: '123456',
    attributes: {
      status: 'paid',
      user_email: 'buyer@example.com',
      user_name: 'Ada',
      created_at: '2026-11-24T10:00:00.000000Z',
      test_mode: false,
      first_order_item: { variant_id: 42 },
      ...over,
    },
  },
});

async function sign(body: string, secret = SECRET): Promise<string> {
  const key = await crypto.subtle.importKey(
    'raw',
    new TextEncoder().encode(secret),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign'],
  );
  const mac = new Uint8Array(await crypto.subtle.sign('HMAC', key, new TextEncoder().encode(body)));
  return [...mac].map((b) => b.toString(16).padStart(2, '0')).join('');
}

async function post(
  body: unknown,
  opts: { secret?: string; path?: string; fetcher?: typeof fetch } = {},
) {
  const text = JSON.stringify(body);
  const req = new Request(`https://hooks.example${opts.path ?? '/webhooks/purchase'}`, {
    method: 'POST',
    body: text,
    headers: { 'X-Signature': await sign(text, opts.secret) },
  });
  const fetcher =
    opts.fetcher ??
    (vi.fn(async () => new Response('{}', { status: 200 })) as unknown as typeof fetch);
  const res = await handle(req, env, fetcher);
  return {
    status: res.status,
    text: await res.text(),
    fetcher: fetcher as unknown as ReturnType<typeof vi.fn>,
  };
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

describe('the webhook', () => {
  it('emails a key for a paid Decay Pro order, the same key on a retry', async () => {
    const first = await post(order());
    expect([first.status, first.text]).toEqual([200, 'sent']);
    const [url, init] = first.fetcher.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('https://api.resend.com/emails');
    expect((init.headers as Record<string, string>)['Idempotency-Key']).toBe('decay-order-123456');
    const mail = JSON.parse(String(init.body));
    expect(mail.to).toBe('buyer@example.com');
    const key = /DECAY-[A-Z2-7-]+/.exec(mail.text)?.[0];
    expect(key).toBeTruthy();
    const again = await post(order());
    const mail2 = JSON.parse(
      String((again.fetcher.mock.calls[0] as [string, RequestInit])[1].body),
    );
    expect(/DECAY-[A-Z2-7-]+/.exec(mail2.text)?.[0]).toBe(key);
  });

  it('refuses a bad signature and answers only its own path', async () => {
    expect((await post(order(), { secret: 'wrong' })).status).toBe(401);
    expect((await post(order(), { path: '/other' })).status).toBe(404);
  });

  it('ignores what is not a paid Decay Pro order (200, nothing sent)', async () => {
    for (const body of [
      { ...order(), meta: { event_name: 'order_refunded' } },
      order({ status: 'pending' }),
      order({ first_order_item: { variant_id: 7 } }),
      order({ user_email: 'not an email' }),
      order({ test_mode: true }),
    ]) {
      const r = await post(body);
      expect(r.status).toBe(200);
      expect(r.text.startsWith('ignored')).toBe(true);
      expect(r.fetcher).not.toHaveBeenCalled();
    }
  });

  it('fails (so Lemon Squeezy retries) when the email cannot be sent', async () => {
    const down = vi.fn(
      async () => new Response('nope', { status: 503 }),
    ) as unknown as typeof fetch;
    const err = vi.spyOn(console, 'error').mockImplementation(() => {});
    const r = await post(order(), { fetcher: down });
    expect(r.status).toBe(500);
    expect(String(err.mock.calls[0]?.[0])).not.toContain('buyer@example.com');
    err.mockRestore();
  });

  it('parses orders defensively', () => {
    expect(parseOrder(null, [])).toEqual({ ignore: 'event undefined' });
    expect('order' in parseOrder(order({ first_order_item: { variant_id: 99 } }), [])).toBe(true);
    expect(parseOrder({ ...order(), data: { ...order().data, id: '1; drop' } }, [])).toEqual({
      ignore: 'malformed order',
    });
  });
});

describe('the email', () => {
  it('carries the key, an activation link and escapes the name', () => {
    const m = licenseMail('Decay <a@b.c>', 'x@y.z', '<b>Eve</b>', 'DECAY-ABCDE');
    expect(m.text).toContain('DECAY-ABCDE');
    expect(m.text).toContain('decay://license/activate?key=DECAY-ABCDE');
    expect(m.html).toContain('&lt;b&gt;Eve&lt;/b&gt;');
    expect(m.html).not.toContain('<b>Eve</b>');
  });
});
