/**
 * Functions injected into web pages by the form probe (P0.7, PRD §7.3).
 *
 * `scripting.executeScript({ func })` serializes each function's source, so every function
 * here must be self-contained: no imports, no references to anything outside its body.
 * They only read the page. Nothing is sent anywhere but back to Decay's service worker,
 * and nothing about what was typed leaves the page — only booleans and counts.
 */

/** Where the typing tracker (`entrypoints/typing.ts`) keeps its state (isolated world). */
export const TYPING_KEY = '__decayTyping_v1';
/** Where the `beforeunload` watcher (`entrypoints/unload-watch.ts`) lives (main world). */
export const UNLOAD_WATCH_KEY = 'decay.unloadWatch.v1';

/** One frame's form state, from the isolated world. */
export interface FrameForms {
  /** The typing tracker ran in this document from the start (it saw every keystroke). */
  tracked: boolean;
  /** A text field holds input the user typed (or, untracked, differs from the page's default). */
  dirty: boolean;
  /**
   * Untracked document with non-empty rich-text editing (contenteditable / designMode):
   * whether the user typed there can't be told.
   */
  unknown: boolean;
}

/** One frame's `beforeunload` state, from the main world. */
export interface FrameUnload {
  /** The watcher ran in this document from the start (it saw `addEventListener` calls). */
  tracked: boolean;
  /** `onbeforeunload` property + `addEventListener('beforeunload')` listeners. */
  handlers: number;
  /** Sticky user activation: without it, browsers never show a "Leave site?" prompt. */
  activated: boolean;
}

/**
 * Isolated world: is there typed text in this frame that closing the tab would lose?
 * Counts text fields, textareas, file pickers and rich-text editors, also inside shadow DOM.
 * Search boxes, checkboxes, radios and selects don't count: losing a filter or a query isn't
 * lost work.
 *
 * Tracked (the typing tracker ran in this document from the start and is still listening):
 * an element the user typed into that still holds text → dirty.
 * Untracked: a field whose value differs from its default → dirty; any other non-empty text
 * field or rich-text editor → unknown (frameworks like React copy the typed value into the
 * default, so "same as default" proves nothing).
 */
export function inspectForms(): FrameForms {
  const NOT_TEXT = new Set([
    'hidden',
    'submit',
    'button',
    'reset',
    'image',
    'search',
    'checkbox',
    'radio',
    'range',
    'color',
  ]);
  const isSearch = (el: Element): boolean => {
    const role = (el.getAttribute('role') ?? '').toLowerCase();
    if (role === 'searchbox') return true;
    if (el.closest('search, [role="search" i]') !== null) return true;
    if (role !== 'combobox') return false;
    // Comboboxes are also autocomplete form fields (address, recipients): only search-like
    // ones are ignored.
    const hint = ['name', 'aria-label', 'placeholder', 'id']
      .map((attr) => el.getAttribute(attr) ?? '')
      .join(' ');
    return /search|query|(^|\s)q(\s|$)/i.test(hint);
  };
  /** A field the user can type into (not disabled / read-only / search / non-text). */
  const textField = (el: Element): el is HTMLInputElement | HTMLTextAreaElement => {
    if (el instanceof HTMLInputElement) {
      if (el.disabled || el.readOnly || isSearch(el)) return false;
      const type = (el.type || 'text').toLowerCase();
      return type === 'file' || !NOT_TEXT.has(type);
    }
    if (el instanceof HTMLTextAreaElement) return !el.disabled && !el.readOnly && !isSearch(el);
    return false;
  };
  const isFile = (el: Element) => el instanceof HTMLInputElement && el.type === 'file';
  const holdsInput = (el: HTMLInputElement | HTMLTextAreaElement) =>
    isFile(el) ? ((el as HTMLInputElement).files?.length ?? 0) > 0 : el.value.trim() !== '';
  const changed = (el: HTMLInputElement | HTMLTextAreaElement) =>
    isFile(el) ? holdsInput(el) : el.value !== el.defaultValue;
  /** Nearest `contenteditable` decides; `designMode` makes the whole document editable. */
  const editableHost = (el: Element): boolean => {
    if (el.ownerDocument?.designMode === 'on') return true;
    const host = el.closest('[contenteditable]');
    if (!host) return false;
    const v = (host.getAttribute('contenteditable') ?? '').toLowerCase();
    return v === '' || v === 'true' || v === 'plaintext-only';
  };
  const hasText = (el: Element) => (el.textContent ?? '').trim() !== '';
  /** Open or closed shadow root (content scripts can open closed ones via `chrome.dom`). */
  const shadowOf = (el: Element): ShadowRoot | null => {
    const dom = (
      globalThis as {
        chrome?: { dom?: { openOrClosedShadowRoot?: (e: Element) => ShadowRoot | null } };
      }
    ).chrome?.dom;
    try {
      return dom?.openOrClosedShadowRoot ? dom.openOrClosedShadowRoot(el) : el.shadowRoot;
    } catch {
      return el.shadowRoot;
    }
  };
  /** `querySelectorAll` that also looks inside shadow roots. */
  const deepQuery = (root: ParentNode, selector: string, out: Element[] = []): Element[] => {
    for (const el of root.querySelectorAll(selector)) out.push(el);
    for (const el of root.querySelectorAll('*')) {
      const inner = shadowOf(el);
      if (inner) deepQuery(inner, selector, out);
    }
    return out;
  };
  const typedText = (root: ParentNode) =>
    deepQuery(root, 'input, textarea').some((f) => textField(f) && holdsInput(f)) ||
    deepQuery(root, '[contenteditable]').some((e) => editableHost(e) && hasText(e));

  const state = (globalThis as Record<string, unknown>).__decayTyping_v1 as
    | { typed?: () => Element[]; alive?: () => boolean }
    | undefined;
  // `document.open()` erases the tracker's listeners (classic iframe editors do this): then it
  // may have missed typing, so the frame counts as untracked.
  const alive = typeof state?.alive === 'function' ? state.alive() : false;
  if (state && typeof state.typed === 'function' && alive) {
    let dirty = false;
    let unknown = false;
    for (const el of state.typed()) {
      if (!el.isConnected) continue;
      if (textField(el)) {
        if (holdsInput(el)) dirty = true;
      } else if (
        el instanceof HTMLInputElement ||
        el instanceof HTMLTextAreaElement ||
        el instanceof HTMLSelectElement
      ) {
        // Search boxes, checkboxes, radios, selects, disabled fields: not lost work.
      } else if (editableHost(el)) {
        if (hasText(el)) dirty = true;
      } else {
        // Typing inside a closed shadow root is reported on its host.
        const inner = shadowOf(el);
        if (inner) {
          if (typedText(inner)) dirty = true;
        } else {
          unknown = true; // typed somewhere we can't look into
        }
      }
      if (dirty) break;
    }
    return { tracked: true, dirty, unknown: !dirty && unknown };
  }

  // Untracked (the page loaded before Decay could watch it).
  let dirty = false;
  let unknown = false;
  for (const el of deepQuery(document, 'input, textarea')) {
    if (!textField(el)) continue;
    if (changed(el)) {
      dirty = true;
      break;
    }
    if (holdsInput(el)) unknown = true;
  }
  if (!dirty && !unknown) {
    unknown =
      document.designMode === 'on'
        ? document.body !== null && hasText(document.body)
        : deepQuery(document, '[contenteditable]').some((e) => editableHost(e) && hasText(e));
  }
  return { tracked: false, dirty, unknown: !dirty && unknown };
}

/** Main world: would closing this frame trigger a "Leave site?" prompt? */
export function inspectUnload(): FrameUnload {
  const w = window as unknown as Record<string | symbol, unknown>;
  const watch = w[Symbol.for('decay.unloadWatch.v1')] as { count?: () => number } | undefined;
  const attr = typeof w.onbeforeunload === 'function' ? 1 : 0;
  const listeners = watch && typeof watch.count === 'function' ? Number(watch.count()) || 0 : 0;
  const activation = (navigator as Navigator & { userActivation?: { hasBeenActive: boolean } })
    .userActivation;
  return {
    tracked: watch !== undefined,
    handlers: attr + listeners,
    activated: activation ? activation.hasBeenActive === true : true,
  };
}
