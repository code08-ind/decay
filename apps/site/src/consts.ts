/** Where people get Decay: the public releases repository (P1.13). */
export const RELEASES = 'https://github.com/code08-ind/decay-releases/releases/latest';
/**
 * Direct downloads of the latest release: fixed-name copies the Release workflow's publish
 * job adds to every release (Decay-mac.dmg, Decay-windows-setup.exe).
 */
export const DOWNLOAD = {
  mac: `${RELEASES}/download/Decay-mac.dmg`,
  windows: `${RELEASES}/download/Decay-windows-setup.exe`,
} as const;
/** The extension's Chrome Web Store listing (Chrome, Edge, Brave, Chromium). */
export const CHROME_STORE =
  'https://chromewebstore.google.com/detail/jidljehgkacgjoefbancbihmdkkpcaaa';
/** The source code (open core: LICENSE.md). */
export const REPO = 'https://github.com/code08-ind/decay';
/** The privacy policy (published on aryangarg.com; Docs/Store/Privacy Policy.md). */
export const PRIVACY = 'https://www.aryangarg.com/decay/privacy-policy';
/** Support (forwarded by Cloudflare Email Routing; also the license email's reply-to). */
export const SUPPORT_EMAIL = 'support.decay@aryangarg.com';
/** Feedback and bug reports. */
export const ISSUES = 'https://github.com/code08-ind/decay-releases/issues';

/** The docs, in reading order (the sidebar and the next / previous links). */
export const DOCS = [
  { href: '/docs', title: 'Getting started' },
  { href: '/docs/how-it-works', title: 'How Decay decides' },
  { href: '/docs/search-and-restore', title: 'Search and restore' },
  { href: '/docs/rules', title: 'Rules: keep, never record, timing' },
  { href: '/docs/windows', title: 'App windows' },
  { href: '/docs/browsers', title: 'Browsers and the extension' },
  { href: '/docs/pro', title: 'Decay Pro' },
  { href: '/docs/privacy', title: 'Privacy and your data' },
  { href: '/docs/troubleshooting', title: 'Troubleshooting' },
] as const;

/**
 * Paddle (merchant of record for Decay Pro, P4.3). The client-side token and the price id
 * are public by design (they can only open a checkout). Sandbox until Paddle approves the
 * live account; then switch all three to the live values (Docs/Release.md "Selling Decay Pro").
 */
export const PADDLE = {
  environment: 'production' as 'sandbox' | 'production',
  clientToken: 'live_cdb7e26dd1d269e4a743fbeafdd',
  priceId: 'pri_01m8bsze9kntjnct8hcx8jdgk8',
};

/**
 * A page's public path. The site builds one file per page (`build.format: 'file'`), so at
 * build time Astro.url.pathname is `/docs/pro.html`; Vercel serves it as `/docs/pro`.
 */
export const cleanPath = (pathname: string) => pathname.replace(/\.html$/, '').replace(/\/(index)?$/, '') || '/';
