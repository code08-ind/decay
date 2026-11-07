/**
 * Typing tracker (P0.7). Registered at runtime (`lib/probe/chrome.ts`) once the user grants
 * site access; runs at `document_start` in Decay's isolated world, in every frame.
 *
 * It only remembers *which* elements received real (trusted) typing — never what was typed —
 * so the form probe (`inspectForms`) can tell a field the user filled in from one a page
 * pre-filled. Nothing leaves the page except the probe's yes/no answer.
 */
export default defineUnlistedScript(() => {
  const KEY = '__decayTyping_v1';
  /** Liveness check: `document.open()` erases window listeners, and then typing goes unseen. */
  const PING = '__decay_typing_ping_v1';
  const g = globalThis as Record<string, unknown>;
  if (g[KEY]) return;
  const MAX = 100;
  const seen = new WeakSet<Element>();
  let refs: WeakRef<Element>[] = [];
  let pong = false;

  const record = (e: Event) => {
    if (!e.isTrusted) return;
    // The real target, also inside open shadow roots (input events are composed).
    const target = e.composedPath()[0] ?? e.target;
    if (!(target instanceof Element) || seen.has(target)) return;
    seen.add(target);
    refs.push(new WeakRef(target));
    if (refs.length > MAX) {
      refs = refs.filter((r) => {
        const el = r.deref();
        if (el?.isConnected) return true;
        if (el) seen.delete(el); // re-attached later (modals, portals): record it again
        return false;
      });
      while (refs.length > MAX) {
        const dropped = refs.shift()?.deref();
        if (dropped) seen.delete(dropped);
      }
    }
  };

  Object.defineProperty(g, KEY, {
    configurable: true,
    value: {
      typed: () => refs.map((r) => r.deref()).filter((el): el is Element => el !== undefined),
      alive: () => {
        pong = false;
        dispatchEvent(new Event(PING));
        return pong;
      },
    },
  });
  addEventListener('input', record, { capture: true, passive: true });
  addEventListener('change', record, { capture: true, passive: true });
  addEventListener(PING, () => {
    pong = true;
  });
});
