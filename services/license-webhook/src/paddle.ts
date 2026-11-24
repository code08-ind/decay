// Paddle Billing webhooks (P4.3). Each notification carries `Paddle-Signature:
// ts=<unix seconds>;h1=<hex>`: the HMAC-SHA256 of `<ts>:<raw body>` with the notification
// destination's secret (several h1 values while a secret is being rotated). A signature
// older than five minutes is refused (a replayed request). Only `transaction.completed`
// for a Decay Pro price gets a license; the buyer's email comes from Paddle's API (the
// event names the customer by id only).

export interface Transaction {
  /** `txn_…`: the license is derived from it, so a retry gives the same key. */
  id: string;
  customerId: string;
  /** Epoch ms the transaction was billed (its `issuedAt`). */
  billedAt: number;
}

export interface Customer {
  email: string;
  name: string;
}

export type Parsed = { transaction: Transaction } | { ignore: string };

/** How old a signature may be. */
export const MAX_AGE_S = 300;

function hex(bytes: ArrayBuffer): string {
  return [...new Uint8Array(bytes)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

/** Constant-time comparison of two strings of the same length. */
function same(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

/** Whether `header` signs `body` with `secret`, within [`MAX_AGE_S`] of `nowS`. */
export async function verifySignature(
  secret: string,
  body: string,
  header: string | null,
  nowS: number,
): Promise<boolean> {
  if (!secret || !header) return false;
  const parts = header.split(';').map((p) => p.trim().split('=', 2) as [string, string?]);
  const ts = parts.find(([k]) => k === 'ts')?.[1];
  const sigs = parts.filter(([k, v]) => k === 'h1' && v).map(([, v]) => (v ?? '').toLowerCase());
  if (!ts || !/^\d{1,12}$/.test(ts) || sigs.length === 0) return false;
  if (Math.abs(nowS - Number(ts)) > MAX_AGE_S) return false;
  const key = await crypto.subtle.importKey(
    'raw',
    new TextEncoder().encode(secret),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign'],
  );
  const mac = hex(await crypto.subtle.sign('HMAC', key, new TextEncoder().encode(`${ts}:${body}`)));
  return sigs.some((s) => same(mac, s));
}

/** What a notification asks for. `prices`: the Decay Pro price ids (empty: any). */
export function parseTransaction(body: unknown, prices: string[]): Parsed {
  // biome-ignore lint/suspicious/noExplicitAny: an untyped webhook body, checked field by field
  const b = body as any;
  const event = b?.event_type;
  if (event !== 'transaction.completed') return { ignore: `event ${String(event)}` };
  const d = b?.data;
  if (d?.status !== 'completed') return { ignore: `status ${String(d?.status)}` };
  const ids: string[] = Array.isArray(d.items)
    ? d.items.map((i: { price?: { id?: unknown } }) => String(i?.price?.id ?? ''))
    : [];
  if (prices.length > 0 && !ids.some((id) => prices.includes(id))) {
    return { ignore: `prices ${ids.join(',')}` };
  }
  const id = String(d.id ?? '');
  const customerId = String(d.customer_id ?? '');
  const billedAt = Date.parse(String(d.billed_at ?? d.created_at ?? ''));
  if (!/^txn_[a-z0-9]{1,60}$/i.test(id) || !/^ctm_[a-z0-9]{1,60}$/i.test(customerId)) {
    return { ignore: 'malformed transaction' };
  }
  if (!Number.isFinite(billedAt)) return { ignore: 'malformed transaction' };
  return { transaction: { id, customerId, billedAt } };
}

const EMAIL = /^[^\s@\p{Cc}]+@[^\s@\p{Cc}]+\.[^\s@\p{Cc}]+$/u;

/** The buyer's email and name, from Paddle's API (`sandbox`: Paddle's test environment). */
export async function fetchCustomer(
  apiKey: string,
  customerId: string,
  sandbox: boolean,
  fetcher: typeof fetch = fetch,
): Promise<Customer> {
  const host = sandbox ? 'https://sandbox-api.paddle.com' : 'https://api.paddle.com';
  const res = await fetcher(`${host}/customers/${encodeURIComponent(customerId)}`, {
    headers: { Authorization: `Bearer ${apiKey}` },
  });
  if (!res.ok) throw new Error(`customer not read: ${res.status}`);
  // biome-ignore lint/suspicious/noExplicitAny: Paddle's response, checked below
  const json = (await res.json()) as any;
  const email = String(json?.data?.email ?? '').trim();
  if (!EMAIL.test(email) || email.length > 254) throw new Error('customer has no usable email');
  return { email, name: String(json?.data?.name ?? '').slice(0, 100) };
}
