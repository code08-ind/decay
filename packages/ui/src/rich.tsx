import { Fragment, isValidElement, type ReactNode } from 'react';
import type { Params } from './i18n';

const OPEN = '';
const CLOSE = '';

/**
 * A message whose parameters include elements ("You reopened <strong>site</strong> 4
 * times"): `format` is the message (a `t` call), elements go where its `{name}` says, so
 * a translation can move them.
 */
export function richText(
  format: (params: Params) => string,
  params: Record<string, ReactNode>,
): ReactNode {
  const elements: ReactNode[] = [];
  const plain: Params = {};
  for (const [name, value] of Object.entries(params)) {
    if (isValidElement(value)) {
      plain[name] = `${OPEN}${elements.length}${CLOSE}`;
      elements.push(value);
    } else {
      plain[name] = value as string | number | null | undefined;
    }
  }
  const parts = format(plain).split(new RegExp(`${OPEN}(\\d+)${CLOSE}`));
  return parts.map((part, i) =>
    i % 2 === 0 ? part : <Fragment key={part}>{elements[Number(part)]}</Fragment>,
  );
}
