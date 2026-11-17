import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { catalogProblems, hardCodedText } from '../../../packages/ui/src/i18n-check';
import { en, TRANSLATIONS } from './strings';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

describe('extension strings', () => {
  it('has well-formed messages', () => {
    expect(catalogProblems(en, TRANSLATIONS)).toEqual([]);
  });

  it('writes no text people see straight into the pages', () => {
    const dirs = ['components', 'entrypoints'].map((d) => join(root, d));
    expect(hardCodedText(dirs, root)).toEqual([]);
  });
});
