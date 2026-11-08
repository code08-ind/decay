// @vitest-environment jsdom

import { implForWrapper } from 'jsdom/lib/generated/idl/utils.js';
import { fireAnEvent } from 'jsdom/lib/jsdom/living/helpers/events.js';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import typingScript from '../../entrypoints/typing';
import unloadWatchScript from '../../entrypoints/unload-watch';
import { inspectForms, inspectUnload, TYPING_KEY, UNLOAD_WATCH_KEY } from './inspect';

const g = globalThis as Record<string, unknown>;

// Real typing fires a *trusted* `input` event, which page scripts can't create. jsdom's
// own internal helper can (it's how jsdom fires events for user actions).
function type(el: HTMLElement, apply: () => void) {
  apply();
  fireAnEvent('input', implForWrapper(el), undefined, { bubbles: true, composed: true });
}

beforeEach(() => {
  document.body.innerHTML = '';
  document.designMode = 'off';
});

describe('inspectForms without the typing tracker (page loaded before Decay watched it)', () => {
  it('is clean for empty fields, search boxes and non-text controls', () => {
    document.body.innerHTML = `
      <input> <textarea></textarea>
      <form role="search"><input name="q"></form>
      <input type="search"> <input role="combobox" aria-label="Search mail">
      <input type="checkbox"> <select><option>a</option></select>`;
    (document.querySelector('[role=combobox]') as HTMLInputElement).value = 'query';
    (document.querySelector('form input') as HTMLInputElement).value = 'query';
    (document.querySelector('input[type=search]') as HTMLInputElement).value = 'query';
    (document.querySelector('input[type=checkbox]') as HTMLInputElement).checked = true;
    expect(inspectForms()).toMatchObject({ tracked: false, dirty: false, unknown: false });
  });

  it('is dirty when a text field or textarea differs from its default', () => {
    document.body.innerHTML = '<input value="a"><textarea>b</textarea>';
    (document.querySelector('textarea') as HTMLTextAreaElement).value = 'b, edited';
    expect(inspectForms()).toMatchObject({ dirty: true });
  });

  it('is unknown for a non-empty field that matches its default (React copies typed text there)', () => {
    document.body.innerHTML = '<input><textarea></textarea>';
    const input = document.querySelector('input') as HTMLInputElement;
    const area = document.querySelector('textarea') as HTMLTextAreaElement;
    input.value = 'half-written ticket';
    input.defaultValue = input.value; // what React DOM does on every change
    expect(inspectForms()).toMatchObject({ tracked: false, dirty: false, unknown: true });
    input.value = '';
    input.defaultValue = '';
    area.value = 'draft';
    area.defaultValue = 'draft';
    expect(inspectForms()).toMatchObject({ dirty: false, unknown: true });
  });

  it('looks inside open shadow roots', () => {
    document.body.innerHTML = '<x-field></x-field>';
    const root = (document.querySelector('x-field') as HTMLElement).attachShadow({ mode: 'open' });
    root.innerHTML = '<input value="a">';
    (root.querySelector('input') as HTMLInputElement).value = 'typed';
    expect(inspectForms()).toMatchObject({ dirty: true });
  });

  it('ignores disabled and read-only fields', () => {
    document.body.innerHTML = '<input disabled><input readonly>';
    for (const el of document.querySelectorAll('input')) el.value = 'x';
    expect(inspectForms().dirty).toBe(false);
  });

  it('is unknown for a non-empty rich-text editor, clean for an empty or disabled one', () => {
    document.body.innerHTML =
      '<div contenteditable="true"></div><div contenteditable="false">x</div>';
    expect(inspectForms()).toMatchObject({ dirty: false, unknown: false });
    document.body.innerHTML = '<div contenteditable>Draft text</div>';
    expect(inspectForms()).toMatchObject({ dirty: false, unknown: true });
  });
});

describe('inspectForms with the typing tracker', () => {
  // jsdom has no navigation timing entry; real pages do. Default: a fresh page load.
  const perf = performance as unknown as { getEntriesByType: (t: string) => unknown[] };
  const originalEntries = perf.getEntriesByType;
  beforeEach(() => {
    perf.getEntriesByType = () => [{ type: 'navigate' }];
    typingScript.main();
  });
  afterEach(() => {
    perf.getEntriesByType = originalEntries;
    delete g[TYPING_KEY];
  });

  it('only counts fields the user really typed into', () => {
    document.body.innerHTML = '<input id="pre" value="a"><input id="typed">';
    // Pre-filled by the page's script: not typed.
    (document.getElementById('pre') as HTMLInputElement).value = 'filled by script';
    expect(inspectForms()).toMatchObject({ tracked: true, dirty: false, unknown: false });
    const typed = document.getElementById('typed') as HTMLInputElement;
    type(typed, () => {
      typed.value = 'my text';
    });
    expect(inspectForms()).toMatchObject({ tracked: true, dirty: true, unknown: false });
  });

  it('ignores untrusted (script-fired) input events', () => {
    document.body.innerHTML = '<input>';
    const input = document.querySelector('input') as HTMLInputElement;
    input.value = 'x';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    expect(inspectForms().dirty).toBe(false);
  });

  it('is clean again once typed text is cleared or its element is removed', () => {
    document.body.innerHTML = '<textarea></textarea><div contenteditable></div>';
    const area = document.querySelector('textarea') as HTMLTextAreaElement;
    const editor = document.querySelector('div') as HTMLDivElement;
    type(area, () => {
      area.value = 'note';
    });
    type(editor, () => {
      editor.textContent = 'reply';
    });
    expect(inspectForms().dirty).toBe(true);
    area.value = '';
    expect(inspectForms().dirty).toBe(true); // the editor still holds text
    editor.remove(); // e.g. a compose box closed after sending
    expect(inspectForms().dirty).toBe(false);
  });

  it('does not flag typed search queries, but does flag autocomplete form fields', () => {
    document.body.innerHTML = '<input role="combobox" name="q"><input role="combobox" name="to">';
    const [search, recipient] = [...document.querySelectorAll('input')] as HTMLInputElement[];
    if (!search || !recipient) throw new Error('fixture');
    type(search, () => {
      search.value = 'cats';
    });
    expect(inspectForms().dirty).toBe(false);
    type(recipient, () => {
      recipient.value = 'ann@example.com';
    });
    expect(inspectForms().dirty).toBe(true);
  });

  it('ignores plain search-like inputs, but not text areas or other fields (QA P0)', () => {
    document.body.innerHTML =
      '<input name="q" placeholder="Search docs"><textarea id="query"></textarea><input name="title">';
    const [q, title] = [...document.querySelectorAll('input')] as HTMLInputElement[];
    const area = document.querySelector('textarea') as HTMLTextAreaElement;
    if (!q || !title) throw new Error('fixture');
    type(q, () => {
      q.value = 'install';
    });
    expect(inspectForms().dirty).toBe(false);
    type(area, () => {
      area.value = 'SELECT * FROM orders';
    });
    expect(inspectForms().dirty).toBe(true); // a long "query" can be real work
    area.value = '';
    type(title, () => {
      title.value = 'Draft';
    });
    expect(inspectForms().dirty).toBe(true);
  });

  it('only treats whole search words as search hints (review P1.1)', () => {
    document.body.innerHTML = `
      <input placeholder="Research topic"> <input id="jquery-title"> <input name="researcher">
      <input id="searchInput"> <input name="site_query">`;
    const inputs = [...document.querySelectorAll('input')] as HTMLInputElement[];
    for (const [i, el] of inputs.entries()) {
      for (const other of inputs) other.value = '';
      type(el, () => {
        el.value = 'typed';
      });
      expect(inspectForms().dirty, el.outerHTML).toBe(i < 3);
    }
  });

  it('stays clean after search typing and checkbox / radio / select changes', () => {
    document.body.innerHTML = `
      <input type="search"> <input type="checkbox"> <input type="radio" name="r">
      <select><option>a</option><option>b</option></select>`;
    const [search, check, radio] = [...document.querySelectorAll('input')] as HTMLInputElement[];
    const select = document.querySelector('select') as HTMLSelectElement;
    if (!search || !check || !radio) throw new Error('fixture');
    type(search, () => {
      search.value = 'cats';
    });
    type(check, () => {
      check.checked = true;
    });
    type(radio, () => {
      radio.checked = true;
    });
    type(select, () => {
      select.selectedIndex = 1;
    });
    expect(inspectForms()).toMatchObject({ tracked: true, dirty: false, unknown: false });
  });

  it('flags typed text even when the page copies it into the default (React)', () => {
    document.body.innerHTML = '<input>';
    const input = document.querySelector('input') as HTMLInputElement;
    type(input, () => {
      input.value = 'my answer';
      input.defaultValue = 'my answer';
    });
    expect(inspectForms()).toMatchObject({ tracked: true, dirty: true, unknown: false });
  });

  it('counts typing inside an open shadow root (reported on the real field)', () => {
    document.body.innerHTML = '<x-editor></x-editor>';
    const host = document.querySelector('x-editor') as HTMLElement;
    const root = host.attachShadow({ mode: 'open' });
    root.innerHTML = '<textarea></textarea>';
    const area = root.querySelector('textarea') as HTMLTextAreaElement;
    type(area, () => {
      area.value = 'note';
    });
    expect(inspectForms().dirty).toBe(true);
  });

  it('counts as untracked when its listeners were erased (document.open)', () => {
    document.body.innerHTML = '<input>';
    const state = g[TYPING_KEY] as { alive: () => boolean };
    expect(state.alive()).toBe(true);
    // Simulate document.open(): the tracker no longer answers its ping.
    Object.defineProperty(g, TYPING_KEY, {
      configurable: true,
      value: { typed: () => [], alive: () => false },
    });
    (document.querySelector('input') as HTMLInputElement).value = 'x';
    expect(inspectForms()).toMatchObject({ tracked: false, dirty: true });
  });

  it('is unknown for untyped changed fields after reload / Back / a discarded tab', () => {
    // Chrome refills fields from the earlier visit after load, without events.
    document.body.innerHTML = '<input value="a">';
    (document.querySelector('input') as HTMLInputElement).value = 'text from an earlier visit';
    expect(inspectForms()).toMatchObject({ dirty: false, unknown: false }); // fresh load
    perf.getEntriesByType = () => [{ type: 'back_forward' }];
    expect(inspectForms()).toMatchObject({ dirty: false, unknown: true });
    perf.getEntriesByType = () => [{ type: 'reload' }];
    expect(inspectForms().unknown).toBe(true);
    perf.getEntriesByType = () => [{ type: 'prerender' }]; // nothing is restored there
    expect(inspectForms().unknown).toBe(false);
    perf.getEntriesByType = () => [{ type: 'navigate' }];
    (document as Document & { wasDiscarded?: boolean }).wasDiscarded = true;
    expect(inspectForms().unknown).toBe(true);
    (document.querySelector('input') as HTMLInputElement).value = 'a'; // back to the default
    expect(inspectForms().unknown).toBe(false);
    delete (document as Document & { wasDiscarded?: boolean }).wasDiscarded;
  });

  it('reports the origins of embedded web pages (never full URLs)', () => {
    document.body.innerHTML =
      '<iframe src="https://pay.example/form?id=7"></iframe><iframe src="about:blank"></iframe><x-host></x-host>';
    const root = (document.querySelector('x-host') as HTMLElement).attachShadow({ mode: 'open' });
    root.innerHTML = '<iframe src="https://chat.example/w"></iframe>';
    expect(inspectForms().frameOrigins?.sort()).toEqual([
      'https://chat.example',
      'https://pay.example',
    ]);
  });

  it('trusts the tracker over a rich-text editor it never saw typing in', () => {
    document.body.innerHTML = '<div contenteditable>Saved document text</div>';
    expect(inspectForms()).toMatchObject({ tracked: true, dirty: false, unknown: false });
  });
});

describe('inspectUnload and the beforeunload watcher', () => {
  const key = Symbol.for(UNLOAD_WATCH_KEY);
  afterEach(() => {
    window.onbeforeunload = null;
  });

  it('sees an onbeforeunload property without the watcher', () => {
    expect(inspectUnload()).toMatchObject({ tracked: false, handlers: 0 });
    window.onbeforeunload = () => 'leave?';
    expect(inspectUnload()).toMatchObject({ tracked: false, handlers: 1 });
  });

  it('counts addEventListener registrations, minus removals and aborts', () => {
    unloadWatchScript.main();
    expect((window as unknown as Record<symbol, unknown>)[key]).toBeDefined();
    const a = () => undefined;
    const b = () => undefined;
    window.addEventListener('beforeunload', a);
    window.addEventListener('beforeunload', a); // duplicate: the browser ignores it
    window.addEventListener('beforeunload', a, true); // capture: a separate registration
    expect(inspectUnload()).toMatchObject({ tracked: true, handlers: 1 });
    window.removeEventListener('beforeunload', a);
    expect(inspectUnload().handlers).toBe(1); // capture one still there
    window.removeEventListener('beforeunload', a, { capture: true });
    expect(inspectUnload().handlers).toBe(0);

    // An already-aborted signal registers nothing.
    window.addEventListener('beforeunload', b, { signal: AbortSignal.abort() });
    expect(inspectUnload().handlers).toBe(0);
    // In vitest the global AbortController is Node's, a different realm than jsdom's
    // EventTarget (like a signal from another frame): the call still goes through and the
    // listener stays counted (protecting is the safe side).
    const ctl = new AbortController();
    window.addEventListener('beforeunload', b, { signal: ctl.signal });
    ctl.abort();
    expect(inspectUnload().handlers).toBe(1);
    window.removeEventListener('beforeunload', b);
    expect(inspectUnload().handlers).toBe(0);
  });

  it('counts uploads in progress (XHR with a file, fetch with a file), not plain requests', async () => {
    unloadWatchScript.main();
    const file = new Blob(['x'.repeat(10)]);
    const xhr = new XMLHttpRequest();
    xhr.open('POST', 'http://localhost:1/upload');
    xhr.send(file);
    expect(inspectUnload().uploads).toBe(1);
    xhr.abort(); // fires loadend
    expect(inspectUnload().uploads).toBe(0);

    const plain = new XMLHttpRequest();
    plain.open('POST', 'http://localhost:1/api');
    plain.send('{"a":1}');
    expect(inspectUnload().uploads).toBe(0);
    plain.abort();

    // send() before open() throws: nothing is running.
    expect(() => new XMLHttpRequest().send(file)).toThrow();
    expect(inspectUnload().uploads).toBe(0);

    const form = new FormData();
    form.append('f', file);
    const pending = fetch('http://localhost:1/up', { method: 'POST', body: form }).catch(
      () => null,
    );
    expect(inspectUnload().uploads).toBe(1);
    await pending;
    await Promise.resolve();
    expect(inspectUnload().uploads).toBe(0);
  });

  it('handles Request uploads, re-opened / aborted XHRs, big strings and binary sizes', async () => {
    unloadWatchScript.main();
    const file = new Blob(['x'.repeat(10)]);
    // Re-opening a running upload ends it without loadend.
    const xhr = new XMLHttpRequest();
    xhr.open('POST', 'http://localhost:1/a');
    xhr.addEventListener('loadend', (e) => e.stopImmediatePropagation());
    xhr.send(file);
    expect(inspectUnload().uploads).toBe(1);
    xhr.open('POST', 'http://localhost:1/b');
    expect(inspectUnload().uploads).toBe(0);
    xhr.send('small');
    expect(inspectUnload().uploads).toBe(0);
    xhr.abort();

    const big = new XMLHttpRequest();
    big.open('POST', 'http://localhost:1/c');
    big.send('x'.repeat(256 * 1024)); // e.g. base64 in JSON
    expect(inspectUnload().uploads).toBe(1);
    big.abort();
    const small = new XMLHttpRequest();
    small.open('POST', 'http://localhost:1/d');
    small.send(new ArrayBuffer(256 * 1024 - 1));
    expect(inspectUnload().uploads).toBe(0);
    small.abort();
    const bin = new XMLHttpRequest();
    bin.open('POST', 'http://localhost:1/e');
    bin.send(new Uint8Array(256 * 1024));
    expect(inspectUnload().uploads).toBe(1);
    bin.abort();
    expect(inspectUnload().uploads).toBe(0);

    // fetch(Request) with a body.
    const req = new Request('http://localhost:1/f', { method: 'POST', body: file });
    const pending = fetch(req).catch(() => null);
    expect(inspectUnload().uploads).toBe(1);
    await pending;
    await Promise.resolve();
    expect(inspectUnload().uploads).toBe(0);
  });

  it('forgets a { once: true } listener after beforeunload fires', () => {
    unloadWatchScript.main();
    const h = () => undefined;
    window.addEventListener('beforeunload', h, { once: true });
    expect(inspectUnload().handlers).toBe(1);
    window.dispatchEvent(new Event('beforeunload'));
    expect(inspectUnload().handlers).toBe(0);
  });

  it('passes every call through and leaves other targets alone', () => {
    unloadWatchScript.main();
    let fired = 0;
    const onClick = () => fired++;
    window.addEventListener('click', onClick);
    window.dispatchEvent(new Event('click'));
    document.body.addEventListener('beforeunload', onClick);
    expect(fired).toBe(1);
    expect(inspectUnload().handlers).toBe(0);
    // A Proxy of the original: same name / length as before (native in a real browser).
    expect(window.addEventListener.length).toBeGreaterThanOrEqual(0);
    window.removeEventListener('click', onClick);
    window.dispatchEvent(new Event('click'));
    expect(fired).toBe(1);
  });
});
