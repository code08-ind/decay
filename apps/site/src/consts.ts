/** Where people get Decay: the public releases repository (P1.13). */
export const RELEASES = 'https://github.com/code08-ind/decay-releases/releases/latest';
/** The source code (open core: LICENSE.md). */
export const REPO = 'https://github.com/code08-ind/decay';
/** The privacy policy (published on aryangarg.com; Docs/Store/Privacy Policy.md). */
export const PRIVACY = 'https://www.aryangarg.com/decay/privacy-policy';
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
