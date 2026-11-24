// The purchase webhook (P4.3, API Design §7.2): Paddle → this Worker → a signed Decay Pro
// license key, emailed to the buyer. Stateless: a transaction's key is derived from the
// transaction (its id and time), so a retried notification issues the same key.
//
// POST /webhooks/purchase   (anything else: 404)
//   401 bad or stale signature · 200 ignored (not a completed Decay Pro purchase)
//   200 sent · 500 failed (Paddle retries non-2xx answers)

import { licenseMail, send } from './email';
import { hexSeed, issue } from './license';
import { fetchCustomer, parseTransaction, verifySignature } from './paddle';

export interface Env {
  /** The notification destination's secret key (Paddle → Developer tools → Notifications). */
  PADDLE_WEBHOOK_SECRET: string;
  /** A Paddle API key that can read customers. */
  PADDLE_API_KEY: string;
  LICENSE_SIGNING_KEY: string;
  RESEND_API_KEY: string;
  EMAIL_FROM: string;
  /** Decay Pro's price ids (`pri_…`, comma-separated); empty: any. */
  PADDLE_PRICE_IDS?: string;
  /** "sandbox" while testing against Paddle's sandbox. */
  PADDLE_ENV?: string;
}

/** The largest body read (a transaction event is a few kB). */
const MAX_BODY = 256 * 1024;

const answer = (status: number, text: string) =>
  new Response(text, { status, headers: { 'Content-Type': 'text/plain; charset=utf-8' } });

export async function handle(
  request: Request,
  env: Env,
  fetcher: typeof fetch = fetch,
  nowS: number = Math.floor(Date.now() / 1000),
): Promise<Response> {
  const url = new URL(request.url);
  if (request.method !== 'POST' || url.pathname !== '/webhooks/purchase') {
    return answer(404, 'not found');
  }
  const body = await request.text();
  if (body.length > MAX_BODY) return answer(413, 'too large');
  const signature = request.headers.get('Paddle-Signature');
  if (!(await verifySignature(env.PADDLE_WEBHOOK_SECRET, body, signature, nowS))) {
    return answer(401, 'bad signature');
  }
  let json: unknown;
  try {
    json = JSON.parse(body);
  } catch {
    return answer(400, 'not JSON');
  }
  const prices = (env.PADDLE_PRICE_IDS ?? '')
    .split(',')
    .map((v) => v.trim())
    .filter(Boolean);
  const parsed = parseTransaction(json, prices);
  if ('ignore' in parsed) return answer(200, `ignored: ${parsed.ignore}`);
  const { transaction: txn } = parsed;
  try {
    const customer = await fetchCustomer(
      env.PADDLE_API_KEY,
      txn.customerId,
      env.PADDLE_ENV === 'sandbox',
      fetcher,
    );
    const key = await issue(
      {
        v: 1,
        email: customer.email,
        tier: 'pro',
        issuedAt: txn.billedAt,
        licenseId: `pd-${txn.id}`,
      },
      hexSeed(env.LICENSE_SIGNING_KEY),
    );
    await send(
      licenseMail(env.EMAIL_FROM, customer.email, customer.name, key),
      env.RESEND_API_KEY,
      `decay-${txn.id}`,
      fetcher,
    );
  } catch (e) {
    // Logged without the key or the email address.
    console.error(`transaction ${txn.id}: ${e instanceof Error ? e.message : String(e)}`);
    return answer(500, 'failed');
  }
  return answer(200, 'sent');
}

export default { fetch: (request: Request, env: Env) => handle(request, env) };
