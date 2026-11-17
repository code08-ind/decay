import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { catalogProblems, hardCodedText } from './i18n-check';
import { en } from './strings';

const here = dirname(fileURLToPath(import.meta.url));

describe('@decay/ui strings', () => {
  it('has well-formed messages', () => {
    expect(catalogProblems(en)).toEqual([]);
  });

  it('writes no text people see straight into the components', () => {
    expect(hardCodedText([here], join(here, '..'))).toEqual([]);
  });
});
