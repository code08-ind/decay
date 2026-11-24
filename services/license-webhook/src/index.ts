// The purchase webhook (P4.3, API Design §7.2): Lemon Squeezy → this Worker → a signed
// Decay Pro license key, emailed to the buyer. Stateless: an order's key is derived from
// the order (its id and time), so a retried webhook issues the same key.
//
// POST /webhooks/purchase   (anything else: 404)
//   401 bad signature · 200 ignored (not a paid Decay Pro order) · 200 sent · 500 failed
//   (Lemon Squeezy retries non-2xx answers).

import { licenseMail, send } from './email';
import { parseOrder, verifySignature } from './lemonsqueezy';
import { hexSeed, issue } from './license';

export interface Env {
  LS_SIGNING_SECRET: string;
  LICENSE_SIGNING_KEY: string;
  RESEND_API_KEY: string;
  EMAIL_FROM: string;
  LS_VARIANT_IDS?: string;
  ALLOW_TEST_MODE?: string;
}

/** The largest body read (an order event is a few kB). */
const MAX_BODY = 256 * 1024;

const answer = (status: number, text: string) =>
  new Response(text, { status, headers: { 'Content-Type': 'text/plain; charset=utf-8' } });

export async function handle(
  request: Request,
  env: Env,
  fetcher: typeof fetch = fetch,
): Promise<Response> {
  const url = new URL(request.url);
  if (request.method !== 'POST' || url.pathname !== '/webhooks/purchase')
    return answer(404, 'not found');
  const body = await request.text();
  if (body.length > MAX_BODY) return answer(413, 'too large');
  if (!(await verifySignature(env.LS_SIGNING_SECRET, body, request.headers.get('X-Signature')))) {
    return answer(401, 'bad signature');
  }
  let json: unknown;
  try {
    json = JSON.parse(body);
  } catch {
    return answer(400, 'not JSON');
  }
  const variants = (env.LS_VARIANT_IDS ?? '')
    .split(',')
    .map((v) => v.trim())
    .filter(Boolean);
  const parsed = parseOrder(json, variants);
  if ('ignore' in parsed) return answer(200, `ignored: ${parsed.ignore}`);
  const { order } = parsed;
  if (order.testMode && env.ALLOW_TEST_MODE !== '1') return answer(200, 'ignored: test mode');
  try {
    const key = await issue(
      {
        v: 1,
        email: order.email,
        tier: 'pro',
        issuedAt: order.createdAt,
        licenseId: `ls-${order.orderId}`,
      },
      hexSeed(env.LICENSE_SIGNING_KEY),
    );
    await send(
      licenseMail(env.EMAIL_FROM, order.email, order.name, key),
      env.RESEND_API_KEY,
      `decay-order-${order.orderId}`,
      fetcher,
    );
  } catch (e) {
    // Logged without the key or the email address.
    console.error(`order ${order.orderId}: ${e instanceof Error ? e.message : String(e)}`);
    return answer(500, 'failed');
  }
  return answer(200, 'sent');
}

export default { fetch: (request: Request, env: Env) => handle(request, env) };
