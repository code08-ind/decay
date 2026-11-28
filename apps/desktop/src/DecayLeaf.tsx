import { type SVGProps, useId } from 'react';

/**
 * The leaf from Decay's app icon (apps/desktop/assets/app-icon.svg): the same shape, tilt and
 * vein, drawn in the current text colour so it can stand in for an icon anywhere in the UI. Always
 * decorative (hidden from screen readers): the word "Decay" sits next to it.
 */
export function DecayLeaf({ size = 24, ...rest }: { size?: number } & SVGProps<SVGSVGElement>) {
  const mask = `decay-leaf-${useId().replace(/[^a-zA-Z0-9_-]/g, '')}`;
  return (
    <svg
      width={size}
      height={size}
      viewBox="160 160 704 704"
      fill="currentColor"
      {...rest}
      aria-hidden="true"
    >
      <mask id={mask}>
        <g transform="rotate(-45 512 512)">
          <path d="M512 196 C 700 330 700 694 512 828 C 324 694 324 330 512 196 Z" fill="#fff" />
          <path d="M512 270 L512 900" stroke="#000" strokeWidth="40" strokeLinecap="round" />
        </g>
      </mask>
      <rect x="160" y="160" width="704" height="704" mask={`url(#${mask})`} />
    </svg>
  );
}
