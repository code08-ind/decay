// The license email (P4.3), sent through Resend's HTTP API. The same purchase always
// sends the same key; Resend's idempotency key keeps a retried webhook from sending it twice.

export interface Mail {
  from: string;
  to: string;
  subject: string;
  text: string;
  html: string;
}

const escapeHtml = (s: string) =>
  s.replace(
    /[&<>"']/g,
    (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c] ?? c,
  );

export function licenseMail(from: string, to: string, name: string, key: string): Mail {
  const link = `decay://license/activate?key=${encodeURIComponent(key)}`;
  const hello = name ? `Hi ${name},` : 'Hi,';
  const text = [
    hello,
    '',
    'Thank you for buying Decay Pro. Your license key:',
    '',
    key,
    '',
    'To activate it, open Decay → Settings → Decay Pro, paste the key and click Activate.',
    `On a computer with Decay installed you can also open this link: ${link}`,
    '',
    'The key works on all your computers and never expires. Keep this email: it is your',
    'only copy of the key. Decay checks it on your computer; nothing is sent anywhere.',
    '',
    'Questions? Reply to this email.',
  ].join('\n');
  const html = `<p>${escapeHtml(hello)}</p>
<p>Thank you for buying Decay Pro. Your license key:</p>
<p style="font-family:ui-monospace,Menlo,Consolas,monospace;word-break:break-all;background:#f3f1ea;padding:12px;border-radius:8px">${escapeHtml(key)}</p>
<p>To activate it, open Decay → Settings → Decay Pro, paste the key and click <b>Activate</b>, or <a href="${escapeHtml(link)}">open it in Decay</a>.</p>
<p>The key works on all your computers and never expires. Keep this email: it is your only copy of the key. Decay checks it on your computer; nothing is sent anywhere.</p>
<p>Questions? Reply to this email.</p>`;
  return { from, to, subject: 'Your Decay Pro license key', text, html };
}

export async function send(
  mail: Mail,
  apiKey: string,
  idempotencyKey: string,
  fetcher: typeof fetch = fetch,
): Promise<void> {
  const res = await fetcher('https://api.resend.com/emails', {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${apiKey}`,
      'Content-Type': 'application/json',
      'Idempotency-Key': idempotencyKey,
    },
    body: JSON.stringify(mail),
  });
  if (!res.ok) throw new Error(`email not sent: ${res.status}`);
}
