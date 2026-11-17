/// <reference path="./node-shim.d.ts" />
// Checks for the catalogs and the sources (tests only; Node). Each package's tests run
// `catalogProblems` on its catalog and `hardCodedText` on its `.tsx` files, so a string
// people see can't be left out of the catalog by accident.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import ts from 'typescript';
import { messageParams, parseMessage } from './i18n';

/** Malformed messages, and translations whose parameters differ from English. */
export function catalogProblems(
  en: Record<string, string>,
  translations: Record<string, Record<string, string | undefined>> = {},
): string[] {
  const problems: string[] = [];
  for (const [key, message] of Object.entries(en)) {
    try {
      parseMessage(message);
    } catch (e) {
      problems.push(`${key}: ${(e as Error).message}`);
    }
  }
  for (const [locale, catalog] of Object.entries(translations)) {
    for (const [key, message] of Object.entries(catalog)) {
      const base = en[key];
      if (message === undefined) continue;
      if (base === undefined) {
        problems.push(`${locale}.${key}: not in English`);
        continue;
      }
      try {
        const a = messageParams(base).join(',');
        const b = messageParams(message).join(',');
        if (a !== b) problems.push(`${locale}.${key}: parameters {${b}} ≠ English {${a}}`);
      } catch (e) {
        problems.push(`${locale}.${key}: ${(e as Error).message}`);
      }
    }
  }
  return problems;
}

/** Attributes and props whose string values people read. */
const TEXT_PROPS = new Set([
  'aria-label',
  'aria-description',
  'aria-roledescription',
  'aria-valuetext',
  'title',
  'placeholder',
  'alt',
  'label',
  'description',
  'text',
  'hint',
  'none',
  'emptyTitle',
  'emptyText',
  'confirmLabel',
  'cancelLabel',
  'heading',
  'subtitle',
  'message',
]);

/** Text that needs no translation: no letters, a catalog key, a single symbol word, or the brand. */
function exempt(text: string): boolean {
  const t = text.replace(/\s+/g, ' ').trim();
  if (!/\p{L}{2,}/u.test(t)) return true;
  if (/^[a-z][\w-]*(\.[\w-]+)+$/.test(t)) return true; // `insights.saveFailed`
  return /^(Decay|Chrome|Edge|Brave|Arc|Firefox|Vivaldi|Opera|GitHub|JSON|CSV|HTML|macOS|Windows|Pro|PRO|Esc|Enter|Tab|Space|Ctrl|Alt|Shift|Cmd|Win|OK)$/.test(
    t,
  );
}

function files(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    if (name === 'node_modules' || name.startsWith('.')) continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) files(p, out);
    else if (/\.tsx?$/.test(name) && !/\.(test|d)\.tsx?$/.test(name)) out.push(p);
  }
  return out;
}

/** Calls whose text arguments people read (toasts, errors shown on the page). */
const TEXT_CALLS = new Set(['notify', 'setError', 'failed', 'fail', 'announce', 'setMessage']);

/** Object properties whose string values people read (option lists, copy objects). */
const TEXT_KEYS = new Set(['label', 'title', 'description', 'hint', 'intro', 'privacy', 'text']);

/** The string literals an expression can evaluate to: `a ? 'x' : 'y'`, `e ?? 'x'`… */
function literals(
  e: ts.Expression,
): (ts.StringLiteral | ts.NoSubstitutionTemplateLiteral | ts.TemplateExpression)[] {
  if (ts.isStringLiteral(e) || ts.isNoSubstitutionTemplateLiteral(e) || ts.isTemplateExpression(e))
    return [e];
  if (ts.isParenthesizedExpression(e)) return literals(e.expression);
  if (ts.isConditionalExpression(e)) return [...literals(e.whenTrue), ...literals(e.whenFalse)];
  if (ts.isBinaryExpression(e)) {
    const op = e.operatorToken.kind;
    if (op === ts.SyntaxKind.QuestionQuestionToken || op === ts.SyntaxKind.BarBarToken)
      return [...literals(e.left), ...literals(e.right)];
    if (op === ts.SyntaxKind.AmpersandAmpersandToken) return literals(e.right);
  }
  return [];
}

function literalText(
  e: ts.StringLiteral | ts.NoSubstitutionTemplateLiteral | ts.TemplateExpression,
): string {
  return ts.isTemplateExpression(e)
    ? [e.head.text, ...e.templateSpans.map((s) => s.literal.text)].join(' ')
    : e.text;
}

/**
 * Text written straight into the screens under `dirs` (`.tsx` and `.ts`): JSX text, text
 * props, string children (also in `?:` / `??`), text arguments of `notify` / `setError` /
 * …, and `label` / `title` / … properties in `.tsx` files. `file:line  text`. A line (or
 * the line before it) with `i18n-ignore` is skipped.
 */
export function hardCodedText(dirs: string[], root: string): string[] {
  const found: string[] = [];
  for (const file of dirs.flatMap((d) => files(d))) {
    const source = readFileSync(file, 'utf8');
    const lines = source.split('\n');
    const tsx = file.endsWith('.tsx');
    const sf = ts.createSourceFile(
      file,
      source,
      ts.ScriptTarget.Latest,
      true,
      tsx ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
    );
    const reportAll = (e: ts.Expression) => {
      for (const l of literals(e)) report(l, literalText(l));
    };
    const report = (node: ts.Node, text: string) => {
      if (exempt(text)) return;
      const line = sf.getLineAndCharacterOfPosition(node.getStart(sf)).line;
      if (/i18n-ignore/.test(lines[line] ?? '') || /i18n-ignore/.test(lines[line - 1] ?? '')) {
        return;
      }
      found.push(`${relative(root, file)}:${line + 1}  ${text.replace(/\s+/g, ' ').trim()}`);
    };
    const visit = (node: ts.Node) => {
      if (ts.isJsxText(node)) report(node, node.text);
      else if (ts.isJsxAttribute(node) && node.initializer) {
        const name = node.name.getText(sf);
        if (TEXT_PROPS.has(name)) {
          const init = node.initializer;
          if (ts.isStringLiteral(init)) report(init, init.text);
          else if (ts.isJsxExpression(init) && init.expression) reportAll(init.expression);
        }
      } else if (
        ts.isJsxExpression(node) &&
        node.expression &&
        (ts.isJsxElement(node.parent) || ts.isJsxFragment(node.parent))
      ) {
        reportAll(node.expression);
      } else if (ts.isCallExpression(node)) {
        const callee = node.expression;
        const name = ts.isIdentifier(callee)
          ? callee.text
          : ts.isPropertyAccessExpression(callee)
            ? callee.name.text
            : '';
        if (TEXT_CALLS.has(name)) for (const a of node.arguments) reportAll(a);
      } else if (
        tsx &&
        ts.isPropertyAssignment(node) &&
        TEXT_KEYS.has(node.name.getText(sf)) &&
        !ts.isJsxAttribute(node.parent)
      ) {
        reportAll(node.initializer);
      }
      ts.forEachChild(node, visit);
    };
    visit(sf);
  }
  return found;
}
