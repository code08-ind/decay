import type { Item } from '../db/types';
import { hostOf } from '../policy/match';
import type { RuleInput } from '../rules/rules';

const MAX_URL_RULE_LENGTH = 2000;

/** What "Always keep" protects, for the button label. */
export function protectTargetFor(item: Pick<Item, 'url'>): {
  kind: 'site' | 'page';
  label: string;
} {
  const host = hostOf(item.url);
  if (host) return { kind: 'site', label: host.replace(/^www\./, '') };
  return { kind: 'page', label: item.url ?? '' };
}

/**
 * "Always keep site": protects the page's host (and its subdomains) — `docs.google.com`,
 * not all of `google.com`. Non-web pages get an exact-URL rule.
 */
export function protectRuleFor(item: Pick<Item, 'url'>): RuleInput | null {
  const host = hostOf(item.url);
  if (host) return { scope: 'domain', match: host.replace(/^www\./, ''), action: 'protect' };
  if (item.url) {
    if (item.url.length > MAX_URL_RULE_LENGTH) {
      return {
        scope: 'urlGlob',
        match: `${item.url.slice(0, MAX_URL_RULE_LENGTH)}*`,
        action: 'protect',
      };
    }
    // An exact-URL glob. A `*` inside the URL would act as a wildcard and protect a bit
    // more than this page — the safe direction for a protect rule.
    return { scope: 'urlGlob', match: item.url, action: 'protect' };
  }
  return null;
}
