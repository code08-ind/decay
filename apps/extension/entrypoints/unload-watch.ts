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
 * It also counts uploads in progress (XHR `send` / `fetch` with file or large bodies) — see
 * below.
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

  // Uploads in progress (PRD §7.3): requests whose body carries files or a large payload.
  // Closing the tab would cancel them. Calls pass through unchanged; only counts are kept.
  // Limits: uploads from Web/Service Workers aren't seen; an upload fetch's promise gets a
  // handler attached (so its rejection no longer reports as "unhandled"); errors thrown by
  // the native methods show this script in their stack.
  const BIG = 256 * 1024;
  const tag = (v: unknown) => Object.prototype.toString.call(v);
  /** Realm-independent checks (bodies may come from an iframe's constructors). */
  const isUpload = (body: unknown): boolean => {
    try {
      if (body === null || body === undefined) return false;
      if (typeof body === 'string') return body.length >= BIG; // e.g. base64 in JSON
      const t = tag(body);
      if (t === '[object Blob]' || t === '[object File]') return (body as Blob).size > 0;
      if (t === '[object FormData]') {
        for (const value of (body as FormData).values()) {
          const vt = tag(value);
          if ((vt === '[object File]' || vt === '[object Blob]') && (value as Blob).size > 0) {
            return true;
          }
          if (typeof value === 'string' && value.length >= BIG) return true;
        }
        return false;
      }
      if (t === '[object ArrayBuffer]') return (body as ArrayBuffer).byteLength >= BIG;
      if (ArrayBuffer.isView(body)) return body.byteLength >= BIG;
      if (t === '[object ReadableStream]') return true;
    } catch {
      // Unknown body types are not counted.
    }
    return false;
  };
  const proxyMethod = (
    proto: object,
    name: string,
    before: (
      self: unknown,
      args: unknown[],
    ) => ((result: unknown, threw: boolean) => void) | undefined,
  ) => {
    const base = Object.getOwnPropertyDescriptor(proto, name);
    if (typeof base?.value !== 'function') return;
    const proxy = new Proxy(base.value as (...a: unknown[]) => unknown, {
      apply(target, self, args: unknown[]) {
        let after: ((result: unknown, threw: boolean) => void) | undefined;
        try {
          after = before(self, args);
        } catch {
          after = undefined;
        }
        let result: unknown;
        try {
          result = Reflect.apply(target, self, args);
        } catch (e) {
          try {
            after?.(undefined, true);
          } catch {
            // Bookkeeping never breaks the page's call.
          }
          throw e;
        }
        try {
          after?.(result, false);
        } catch {
          // Bookkeeping never breaks the page's call.
        }
        return result;
      },
    });
    Object.defineProperty(proto, name, { ...base, value: proxy });
  };

  // XMLHttpRequest: an upload XHR stays in this set while its readyState says a request is
  // running (1 OPENED with send() called, 2, 3). The count is read from the live readyState,
  // so a missed `loadend` (re-open, stopped propagation) can't leave it stuck.
  const xhrs = new Set<XMLHttpRequest>();
  if (typeof XMLHttpRequest !== 'undefined') {
    const proto = XMLHttpRequest.prototype;
    proxyMethod(proto, 'send', (xhr, [body]) => {
      if (!isUpload(body) || tag(xhr) !== '[object XMLHttpRequest]') return undefined;
      return (_result, threw) => {
        if (!threw) xhrs.add(xhr as XMLHttpRequest);
      };
    });
    // open() on a running request ends it without `loadend`: forget it.
    proxyMethod(proto, 'open', (xhr) => {
      xhrs.delete(xhr as XMLHttpRequest);
      return undefined;
    });
    proxyMethod(proto, 'abort', (xhr) => (_r, threw) => {
      if (!threw) xhrs.delete(xhr as XMLHttpRequest);
    });
  }
  const runningXhrUploads = () => {
    let n = 0;
    for (const xhr of xhrs) {
      try {
        if (xhr.readyState >= 1 && xhr.readyState <= 3) n++;
        else xhrs.delete(xhr);
      } catch {
        xhrs.delete(xhr);
      }
    }
    return n;
  };

  // fetch: counted until the response arrives or the request fails. The page's own promise
  // is returned untouched (the native `then` is used to observe it). keepalive uploads are
  // counted too, briefly over-protecting: harmless.
  const nativeThen = Promise.prototype.then;
  let fetchUploads = 0;
  const fetchHost = Object.hasOwn(window, 'fetch') ? window : Window.prototype;
  proxyMethod(fetchHost, 'fetch', (_self, [input, init]) => {
    const initBody = (init as RequestInit | undefined)?.body;
    let upload = isUpload(initBody);
    if (
      !upload &&
      (initBody === undefined || initBody === null) &&
      tag(input) === '[object Request]'
    ) {
      // A Request object carries its body as a stream: any request with a body counts.
      const req = input as Request;
      upload = req.body !== null && !/^(GET|HEAD)$/i.test(req.method);
    }
    if (!upload) return undefined;
    fetchUploads++;
    let counted = true;
    const done = () => {
      if (counted) fetchUploads--;
      counted = false;
    };
    return (result, threw) => {
      if (!threw && result && typeof (result as Promise<unknown>).then === 'function') {
        Reflect.apply(nativeThen, result, [done, done]);
      } else {
        done();
      }
    };
  });

  Object.defineProperty(w, KEY, {
    value: {
      count: () => listeners.size,
      uploads: () => runningXhrUploads() + Math.max(0, fetchUploads),
    },
  });
});
