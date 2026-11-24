// Lemon Squeezy webhooks (P4.3): the `X-Signature` header is the hex HMAC-SHA256 of the
// raw body with the webhook's signing secret. Only `order_created` for a paid order (of a
// Decay Pro variant, when configured) gets a license.

export interface Order {
  orderId: string;
  email: string;
  name: string;
  /** Epoch ms of the order (the license's `issuedAt`: the same order always gives the same key). */
  createdAt: number;
  testMode: boolean;
}

export type Parsed = { order: Order } | { ignore: string };

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

export async function verifySignature(
  secret: string,
  body: string,
  signature: string | null,
): Promise<boolean> {
  if (!secret || !signature) return false;
  const key = await crypto.subtle.importKey(
    'raw',
    new TextEncoder().encode(secret),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign'],
  );
  const mac = hex(await crypto.subtle.sign('HMAC', key, new TextEncoder().encode(body)));
  return same(mac, signature.trim().toLowerCase());
}

const EMAIL = /^[^\s@\p{Cc}]+@[^\s@\p{Cc}]+\.[^\s@\p{Cc}]+$/u;

/** What a webhook body asks for. `variants`: the Decay Pro variant ids (empty: any). */
export function parseOrder(body: unknown, variants: string[]): Parsed {
  // biome-ignore lint/suspicious/noExplicitAny: an untyped webhook body, checked field by field
  const b = body as any;
  const event = b?.meta?.event_name;
  if (event !== 'order_created') return { ignore: `event ${String(event)}` };
  const a = b?.data?.attributes;
  if (b?.data?.type !== 'orders' || !a) return { ignore: 'not an order' };
  if (a.status !== 'paid') return { ignore: `status ${String(a.status)}` };
  const variant = String(a.first_order_item?.variant_id ?? '');
  if (variants.length > 0 && !variants.includes(variant)) return { ignore: `variant ${variant}` };
  const email = String(a.user_email ?? '').trim();
  if (!EMAIL.test(email) || email.length > 254) return { ignore: 'no usable email' };
  const createdAt = Date.parse(String(a.created_at ?? ''));
  const orderId = String(b.data.id ?? '');
  if (!Number.isFinite(createdAt) || !/^\d{1,20}$/.test(orderId))
    return { ignore: 'malformed order' };
  return {
    order: {
      orderId,
      email,
      name: String(a.user_name ?? '').slice(0, 100),
      createdAt,
      testMode: a.test_mode === true,
    },
  };
}
