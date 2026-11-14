// Chunking (Docs/API Design.md §4.3), the same format as the Rust `decay-bridge::chunk`:
// a message whose JSON is too big is serialised and cut into pieces; each piece travels as
// a string payload `{v, id, type, chunk: {seq, total, groupId}, payload: "<piece>"}` and
// the receiver joins them in `seq` order.
//
// Pieces are cut at code point boundaries — never between the two halves of a surrogate
// pair — because a lone surrogate in a piece becomes U+FFFD on the way (an emoji split
// across chunks would arrive broken). Each piece's escaped length stays within 900 KB.

import { type Chunk, type Envelope, newId, PROTOCOL_VERSION } from './protocol';

/** Budget for one chunk's escaped piece. */
export const CHUNK_BYTES = 900 * 1024;
/** Messages up to this size go whole. */
export const MAX_FRAME = 1024 * 1024;
export const REASSEMBLY_TIMEOUT_MS = 10_000;
export const MAX_CHUNKS = 160;
export const MAX_GROUPS = 64;
export const MAX_GROUP_ID_LEN = 64;
export const MAX_PENDING_BYTES = 160 * 1024 * 1024;

const encoder = new TextEncoder();

/** Bytes a code point takes inside a JSON string (UTF-8, with JSON's escapes). */
function escapedLen(cp: number): number {
  if (cp === 0x22 || cp === 0x5c || cp === 0x0a || cp === 0x0d || cp === 0x09) return 2;
  if (cp === 0x08 || cp === 0x0c) return 2;
  if (cp < 0x20) return 6;
  if (cp >= 0xd800 && cp <= 0xdfff) return 6; // a lone surrogate: JSON.stringify escapes it
  if (cp < 0x80) return 1;
  if (cp < 0x800) return 2;
  if (cp < 0x10000) return 3;
  return 4;
}

export class SplitError extends Error {}

/** `env` as frames: itself if its JSON fits `maxFrame`, else chunks of its JSON. */
export function splitEnvelope(
  env: Envelope,
  maxFrame = MAX_FRAME,
  chunkBytes = CHUNK_BYTES,
): Envelope[] {
  const json = JSON.stringify(env);
  if (encoder.encode(json).length <= maxFrame) return [env];
  if (encoder.encode(env.type).length > 64) throw new SplitError('message type too long to chunk');
  const pieces: string[] = [];
  let start = 0;
  let budget = 0;
  let i = 0;
  while (i < json.length) {
    const cp = json.codePointAt(i) ?? 0;
    const width = cp > 0xffff ? 2 : 1; // UTF-16 units: a pair stays together
    const len = escapedLen(cp);
    if (budget + len > chunkBytes) {
      pieces.push(json.slice(start, i));
      start = i;
      budget = 0;
      // Too big already: stop scanning (the receiver would refuse it anyway).
      if (pieces.length >= MAX_CHUNKS) {
        throw new SplitError(`message needs more than ${MAX_CHUNKS} chunks`);
      }
    }
    budget += len;
    i += width;
  }
  pieces.push(json.slice(start));
  if (pieces.length > MAX_CHUNKS) {
    throw new SplitError(`message needs ${pieces.length} chunks, over ${MAX_CHUNKS}`);
  }
  const groupId = newId();
  return pieces.map((piece, seq) => ({
    v: PROTOCOL_VERSION,
    id: newId(),
    type: env.type,
    chunk: { seq, total: pieces.length, groupId },
    payload: piece,
  }));
}

interface Group {
  type: string;
  parts: (string | undefined)[];
  received: number;
  bytes: number;
  startedAt: number;
}

/** Joins chunk groups back into envelopes (app → extension direction). */
export class Reassembler {
  private groups = new Map<string, Group>();
  private pending = 0;

  private drop(id: string): void {
    const g = this.groups.get(id);
    if (g) {
      this.pending -= g.bytes;
      this.groups.delete(id);
    }
  }

  expire(now: number): void {
    for (const [id, g] of this.groups) {
      if (now - g.startedAt > REASSEMBLY_TIMEOUT_MS) this.drop(id);
    }
  }

  /**
   * An unchunked envelope comes straight back; a chunk is held until its group is
   * complete, then the original envelope is returned. Invalid chunks throw (and drop
   * their group).
   */
  push(env: Envelope, now: number): Envelope | null {
    this.expire(now);
    const chunk: Chunk | undefined = env.chunk;
    if (!chunk) return env;
    const { seq, total, groupId } = chunk;
    const fail = (why: string): never => {
      this.drop(groupId);
      throw new Error(why);
    };
    if (
      !Number.isInteger(seq) ||
      !Number.isInteger(total) ||
      total < 1 ||
      total > MAX_CHUNKS ||
      seq < 0 ||
      seq >= total
    ) {
      fail('chunk out of range');
    }
    if (typeof env.payload !== 'string') fail('chunk payload is not text');
    if (typeof groupId !== 'string' || groupId.length > MAX_GROUP_ID_LEN) fail('bad chunk group');
    const piece = env.payload as string;
    let group = this.groups.get(groupId);
    const overhead = group ? 0 : 1024 + 8 * total;
    if (
      (!group && this.groups.size >= MAX_GROUPS) ||
      this.pending + overhead + piece.length > MAX_PENDING_BYTES
    ) {
      fail('too much chunked data');
    }
    if (!group) {
      group = {
        type: env.type,
        parts: new Array(total).fill(undefined),
        received: 0,
        bytes: overhead,
        startedAt: now,
      };
      this.groups.set(groupId, group);
      this.pending += overhead;
    }
    if (group.parts.length !== total || group.type !== env.type) fail('inconsistent chunk group');
    if (group.parts[seq] === undefined) {
      group.parts[seq] = piece;
      group.received++;
      group.bytes += piece.length;
      this.pending += piece.length;
    }
    if (group.received < total) return null;
    this.drop(groupId);
    let original: unknown;
    try {
      original = JSON.parse(group.parts.join(''));
    } catch {
      throw new Error('reassembled message is invalid');
    }
    const out = original as Envelope;
    if (typeof out !== 'object' || out === null || out.chunk || out.type !== group.type) {
      throw new Error('nested or mismatched chunk');
    }
    return out;
  }
}
