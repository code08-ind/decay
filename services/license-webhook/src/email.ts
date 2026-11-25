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

/** The site that hosts the logo and the activation page (apps/site). */
const SITE = 'https://decay.aryangarg.com';

// Decay's light theme (packages/ui/tokens.css). Mail apps ignore stylesheets and web fonts
// unevenly, so the HTML uses tables and inline styles with system fallbacks.
const C = {
  canvas: '#f7f3e8',
  surface: '#fffdf7',
  border: '#e6dfcb',
  text: '#1d2219',
  muted: '#5d6253',
  faint: '#8a8d7c',
  accentSoft: '#e9f0dc',
  primary: '#1d2219',
  primaryText: '#ffffff',
};
const SERIF = "'Newsreader',Georgia,'Times New Roman',serif";
const SANS = "'DM Sans',-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif";
const MONO = "'DM Mono',ui-monospace,Menlo,Consolas,monospace";

export function licenseMail(from: string, to: string, name: string, key: string): Mail {
  // Gmail and others strip decay:// links, so the button opens a page on the site that hands
  // the key to the app; the key rides in the fragment, which never reaches the server.
  const link = `${SITE}/activate#key=${encodeURIComponent(key)}`;
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
    '',
    `— Decay · ${SITE}`,
  ].join('\n');
  const p = `margin:0 0 16px;font-family:${SANS};font-size:15px;line-height:1.6;color:${C.text}`;
  const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="color-scheme" content="light">
<title>Your Decay Pro license key</title>
</head>
<body style="margin:0;padding:0;background:${C.canvas}">
<div style="display:none;max-height:0;overflow:hidden">Your Decay Pro license key, and how to activate it.</div>
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="background:${C.canvas}">
<tr><td align="center" style="padding:32px 16px">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="max-width:560px">
<tr><td style="padding:0 4px 20px">
<table role="presentation" cellpadding="0" cellspacing="0" border="0"><tr>
<td style="vertical-align:middle"><img src="${SITE}/icon.png" width="36" height="36" alt="" style="display:block;border:0;border-radius:9px"></td>
<td style="vertical-align:middle;padding-left:10px;font-family:${SERIF};font-size:24px;color:${C.text}">Decay</td>
</tr></table>
</td></tr>
<tr><td style="background:${C.surface};border:1px solid ${C.border};border-radius:16px;padding:32px">
<h1 style="margin:0 0 20px;font-family:${SERIF};font-size:28px;font-weight:500;line-height:1.25;color:${C.text}">Thank you for buying Decay Pro</h1>
<p style="${p}">${escapeHtml(hello)}</p>
<p style="${p}">Here is your license key:</p>
<div style="margin:0 0 24px;padding:16px;background:${C.accentSoft};border-radius:10px;font-family:${MONO};font-size:13px;line-height:1.6;color:${C.text};word-break:break-all">${escapeHtml(key)}</div>
<table role="presentation" cellpadding="0" cellspacing="0" border="0" style="margin:0 0 24px"><tr>
<td style="background:${C.primary};border-radius:999px"><a href="${escapeHtml(link)}" style="display:inline-block;padding:12px 24px;font-family:${SANS};font-size:15px;font-weight:600;color:${C.primaryText};text-decoration:none">Activate in Decay</a></td>
</tr></table>
<p style="${p}">Or open <b>Decay → Settings → Decay Pro</b>, paste the key and click <b>Activate</b>.</p>
<p style="${p}">The key works on all your computers and never expires. Keep this email: it is your only copy of the key. Decay checks it on your computer; nothing is sent anywhere.</p>
<p style="${p};margin-bottom:0">Questions? Just reply to this email.</p>
</td></tr>
<tr><td align="center" style="padding:20px 4px 0;font-family:${SANS};font-size:12px;line-height:1.6;color:${C.faint}">
Decay quietly archives the tabs and windows you stopped using.<br>
<a href="${SITE}" style="color:${C.muted}">decay.aryangarg.com</a>
</td></tr>
</table>
</td></tr>
</table>
</body>
</html>`;
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
