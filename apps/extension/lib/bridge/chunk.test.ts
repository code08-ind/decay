import { describe, expect, it } from 'vitest';
import {
  CHUNK_BYTES,
  MAX_CHUNKS,
  MAX_FRAME,
  MAX_GROUPS,
  Reassembler,
  splitEnvelope,
} from './chunk';
import { envelope } from './protocol';

const encoder = new TextEncoder();
const bytes = (v: unknown) => encoder.encode(JSON.stringify(v)).length;

describe('chunking', () => {
  it('leaves small messages whole', () => {
    const env = envelope('snapshot', { a: 1 });
    expect(splitEnvelope(env)).toEqual([env]);
  });

  it('never splits an emoji, and every chunk fits the limit', () => {
    // Emoji (surrogate pairs), quotes and control characters all grow when escaped again.
    const title = '😀"\\\u0001é ';
    const env = envelope('import.archive', { titles: Array(150_000).fill(title) });
    const chunks = splitEnvelope(env);
    expect(chunks.length).toBeGreaterThan(2);
    for (const c of chunks) {
      expect(bytes(c)).toBeLessThanOrEqual(MAX_FRAME);
      const piece = c.payload as string;
      // No piece starts with a low surrogate or ends with a high one.
      expect(/^[\udc00-\udfff]/.test(piece)).toBe(false);
      expect(/[\ud800-\udbff]$/.test(piece)).toBe(false);
      expect(JSON.stringify(piece)).not.toMatch(/\\ud[89ab]/i);
    }
    const r = new Reassembler();
    let out = null;
    for (const c of [...chunks].reverse()) out = r.push(c, 0);
    expect(out).toEqual(env);
  });

  it('keeps pieces within the escaped budget', () => {
    const env = envelope('x', { q: '"'.repeat(2_000_000) });
    for (const c of splitEnvelope(env)) {
      expect(bytes(c.payload)).toBeLessThanOrEqual(CHUNK_BYTES + 2);
    }
  });

  it('refuses messages that need too many chunks', () => {
    // Small chunks, same rule: more than MAX_CHUNKS pieces is refused (fast, even under
    // coverage; a real-size message would be ~145 MB).
    const env = envelope('x', { q: 'a'.repeat((MAX_CHUNKS + 1) * 100) });
    expect(() => splitEnvelope(env, 100, 100)).toThrow(/chunks/);
    const fits = envelope('x', { q: 'a'.repeat((MAX_CHUNKS - 2) * 100) });
    expect(splitEnvelope(fits, 100, 100).length).toBeLessThanOrEqual(MAX_CHUNKS);
  });

  it('rejects hostile chunks and caps groups', () => {
    const chunk = (seq: number, total: number, payload: unknown, groupId = 'g') => ({
      v: 1,
      id: 'i',
      type: 'snapshot',
      chunk: { seq, total, groupId },
      payload,
    });
    const r = new Reassembler();
    expect(() => r.push(chunk(3, 3, 'x'), 0)).toThrow(/range/);
    expect(() => r.push(chunk(0, MAX_CHUNKS + 1, 'x'), 0)).toThrow(/range/);
    expect(() => r.push(chunk(0, 1, 5), 0)).toThrow(/text/);
    expect(() => r.push(chunk(0, 1, 'nope'), 0)).toThrow(/invalid/);
    for (let i = 0; i < MAX_GROUPS; i++) expect(r.push(chunk(0, 2, '', `g${i}`), 0)).toBeNull();
    expect(() => r.push(chunk(0, 2, '', 'one-more'), 0)).toThrow(/too much/);
    r.expire(60_000);
    expect(r.push(chunk(0, 2, '{', 'fresh'), 60_000)).toBeNull();
  });
});
