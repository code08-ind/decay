// The docs search index (/search.json), built with the site: one entry per docs section
// (page intro + every heading), with plain text for matching. The ⌘K dialog in
// components/Search.astro fetches it the first time it opens.
import type { MarkdownInstance } from 'astro';
import { DOCS } from '../consts';

type Section = { page: string; title: string; heading: string; href: string; text: string };

/** Markdown → plain text, good enough for matching and snippets. */
function plain(md: string): string {
  return md
    .replace(/```[\s\S]*?```/g, (block) => block.replace(/```\w*/g, ' '))
    .replace(/<[^>]+>/g, ' ')
    .replace(/!\[[^\]]*\]\([^)]*\)/g, ' ')
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/^\s*\|?\s*:?-{3,}.*$/gm, ' ')
    .replace(/[|*_`>#]/g, ' ')
    .replace(/^\s*(?:[-+]|\d+\.)\s+/gm, '')
    .replace(/\s+/g, ' ')
    .trim();
}

export async function GET() {
  const pages = import.meta.glob<MarkdownInstance<{ title: string }>>('./docs/*.md', { eager: true });
  const sections: Section[] = [];

  for (const [file, md] of Object.entries(pages)) {
    const slug = file.replace('./docs/', '').replace(/\.md$/, '');
    const href = slug === 'index' ? '/docs' : `/docs/${slug}`;
    const page = DOCS.find((d) => d.href === href)?.title ?? md.frontmatter.title;
    const headings = md.getHeadings().filter((h) => h.depth >= 2 && h.depth <= 3);

    // Split the source at its ## / ### lines; they come in the same order as getHeadings().
    const parts = md.rawContent().split(/^#{2,3}\s+.*$/m);
    const intro = parts[0].replace(/^#\s+.*$/m, '');
    sections.push({ page, title: page, heading: '', href, text: plain(intro) });
    headings.forEach((h, i) => {
      sections.push({ page, title: h.text, heading: h.text, href: `${href}#${h.slug}`, text: plain(parts[i + 1] ?? '') });
    });
  }

  // Docs reading order, then the order inside each page.
  const order = (s: Section) => DOCS.findIndex((d) => s.href.split('#')[0] === d.href);
  sections.sort((a, b) => order(a) - order(b));

  return new Response(JSON.stringify(sections), { headers: { 'Content-Type': 'application/json' } });
}
