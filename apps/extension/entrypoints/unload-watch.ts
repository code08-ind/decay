/**
 * `beforeunload` watcher (P0.7). Registered at runtime (`lib/probe/chrome.ts`) once the user
 * grants site access; runs at `document_start` in the page's main world, in every frame.
 *
 * Browsers expose no way to ask whether a page has `beforeunload` listeners, so this counts
 * `window.addEventListener('beforeunload', …)` registrations (minus removals) for the probe's
 * `inspectUnload`. Every call is passed through unchanged, to whatever the prototype chain
 * holds *at call time* — so libraries that patch `EventTarget.prototype` later (zone.js,
 * monitoring agents) still see every call.
 *
 * Trade-off: the page can notice the wrapper (an own `addEventListener` on Window.prototype
 * and the `Symbol.for` key). The wrappers are Proxies of the native functions, so `name`,
 * `length` and `toString()` look native.
 */
export default defineUnlistedScript(() => {
  const KEY = Symbol.for('decay.unloadWatch.v1');
  const w = window as unknown as Record<symbol, unknown>;
  if (w[KEY]) return;

  /** listener → bit 1 (bubble) | bit 2 (capture): the browser dedupes on (listener, capture). */
  const listeners = new Map<unknown, number>();
  const captureBit = (options: unknown) =>
    (typeof options === 'boolean' ? options : Boolean((options as { capture?: unknown })?.capture))
      ? 2
      : 1;
  const drop = (listener: unknown, bit: number) => {
    const bits = (listeners.get(listener) ?? 0) & ~bit;
    if (bits) listeners.set(listener, bits);
    else listeners.delete(listener);
    once.get(listener)?.delete(bit);
  };
  /**
   * `{ once: true }` listeners are removed by the browser after they fire, without a
   * `removeEventListener` call. While any exist, a capture listener of ours (registered only
   * then, so ordinary pages never get one) forgets them whenever beforeunload is dispatched.
   */
  const once = new Map<unknown, Set<number>>();
  let sweeping = false;
  const forgetOnce = () => {
    for (const [listener, bits] of once) for (const bit of bits) drop(listener, bit);
    once.clear();
    if (sweeping) {
      sweeping = false;
      winRemove.call(window, 'beforeunload', forgetOnce, true);
    }
  };
  // For abort signals (we run at document_start, before any page script can patch it).
  const listen = EventTarget.prototype.addEventListener;
  // Window's own methods as they are before wrapping (for our once-listener sweeper).
  const winAdd = window.addEventListener;
  const winRemove = window.removeEventListener;
  /** Unqualified calls (`addEventListener(…)`) reach the window with `this` undefined. */
  const isWindow = (self: unknown) => self === window || self === undefined || self === null;

  const onAdd = (self: unknown, [type, listener, options]: unknown[]) => {
    if (!isWindow(self) || type !== 'beforeunload' || !listener) return;
    const signal = (options as { signal?: AbortSignal } | undefined)?.signal;
    if (signal?.aborted) return;
    const bit = captureBit(options);
    listeners.set(listener, (listeners.get(listener) ?? 0) | bit);
    if ((options as { once?: unknown } | undefined)?.once === true) {
      const bits = once.get(listener) ?? new Set<number>();
      bits.add(bit);
      once.set(listener, bits);
      if (!sweeping) {
        sweeping = true;
        winAdd.call(window, 'beforeunload', forgetOnce, true);
      }
    }
    if (signal && typeof signal.addEventListener === 'function') {
      listen.call(signal, 'abort', () => drop(listener, bit), { once: true });
    }
  };
  const onRemove = (self: unknown, [type, listener, options]: unknown[]) => {
    if (isWindow(self) && type === 'beforeunload' && listener) drop(listener, captureBit(options));
  };

  // Defined where `window` finds the method first (own property in some engines), else on
  // Window.prototype — never on EventTarget.prototype, so other targets are untouched.
  const wrap = (
    name: 'addEventListener' | 'removeEventListener',
    track: (self: unknown, args: unknown[]) => void,
  ) => {
    let holder: object | null = window;
    while (holder && !Object.hasOwn(holder, name)) holder = Object.getPrototypeOf(holder);
    const host = holder && holder !== EventTarget.prototype ? holder : Window.prototype;
    const base = Object.getOwnPropertyDescriptor(holder ?? EventTarget.prototype, name);
    // What `window[name]` resolves to now (the property may be an accessor in some engines).
    const native = window[name] as unknown as (...a: unknown[]) => unknown;
    const proxy: (...a: unknown[]) => unknown = new Proxy(native, {
      apply(target, self, args: unknown[]) {
        try {
          track(self, args);
        } catch {
          // Bookkeeping must never break the page's own call (e.g. a signal from another frame).
        }
        // The next implementation up the chain *now* (it may have been patched since).
        const parent = host === holder ? null : Object.getPrototypeOf(host);
        const next = parent ? (Reflect.get(parent, name) as typeof native) : target;
        return Reflect.apply(next === proxy ? target : next, self ?? window, args);
      },
    });
    Object.defineProperty(host, name, {
      configurable: base?.configurable ?? true,
      enumerable: base?.enumerable ?? true,
      writable: base?.writable ?? true,
      value: proxy,
    });
  };
  wrap('addEventListener', onAdd);
  wrap('removeEventListener', onRemove);
  Object.defineProperty(w, KEY, { value: { count: () => listeners.size } });
});
