import { describe, expect, it } from 'vitest';
import {
  Bridge,
  type BridgeDeps,
  type BridgeState,
  HANDSHAKE_TIMEOUT_MS,
  type PortLike,
  STUCK_ATTEMPT_MS,
} from './connection';
import { type Envelope, envelope, kinds } from './protocol';

/** The value, or a test failure when it's missing. */
function must<T>(value: T | undefined | null): T {
  if (value === undefined || value === null) throw new Error('missing');
  return value;
}

/** A fake native port: records what the extension sends, and lets the test answer. */
class FakePort implements PortLike {
  sent: Envelope[] = [];
  disconnected = false;
  private messageFns: ((m: unknown) => void)[] = [];
  private disconnectFns: (() => void)[] = [];
  postMessage(m: unknown) {
    this.sent.push(JSON.parse(JSON.stringify(m)));
  }
  disconnect() {
    this.disconnected = true;
  }
  onMessage = { addListener: (fn: (m: unknown) => void) => this.messageFns.push(fn) };
  onDisconnect = { addListener: (fn: () => void) => this.disconnectFns.push(fn) };
  receive(m: unknown) {
    for (const fn of this.messageFns) fn(m);
  }
  drop() {
    for (const fn of this.disconnectFns) fn();
  }
}

function setup(opts: { noPermission?: boolean } = {}) {
  const ports: FakePort[] = [];
  const states: BridgeState[] = [];
  const messages: Envelope[] = [];
  const timers = new Map<number, () => void>();
  let nextTimer = 1;
  const repeating = new Map<number, () => void>();
  const deps: BridgeDeps = {
    connectNative: () => {
      if (opts.noPermission) throw new Error('Access to native messaging requires permission');
      const p = new FakePort();
      ports.push(p);
      return p;
    },
    hello: async () => ({
      extVersion: '1.2.0',
      browser: 'chrome',
      profileId: 'p1',
      capabilities: ['probe'],
    }),
    onMessage: (e) => messages.push(e),
    onState: (s) => states.push(s),
    now: () => 0,
    setTimer: (fn) => {
      timers.set(nextTimer, fn);
      return nextTimer++;
    },
    clearTimer: (h) => timers.delete(h as number),
    setRepeating: (fn) => {
      repeating.set(nextTimer, fn);
      return nextTimer++;
    },
    clearRepeating: (h) => repeating.delete(h as number),
  };
  const fire = (all: Map<number, () => void>) => {
    for (const [id, fn] of [...all]) {
      if (all === timers) timers.delete(id);
      fn();
    }
  };
  return {
    bridge: new Bridge(deps),
    deps,
    ports,
    states,
    messages,
    fireTimers: () => fire(timers),
    fireHeartbeat: () => fire(repeating),
  };
}

const welcomeFor = (hello: Envelope) =>
  envelope(
    kinds.welcome,
    {
      coreVersion: '0.1.0',
      minExtVersion: '1.0.0',
      mode: 'shadow',
      config: { probeEnabled: true, heartbeatSec: 20 },
    },
    hello.id,
  );

describe('Bridge', () => {
  it('says hello with the launch flag and connects on welcome', async () => {
    const t = setup();
    await t.bridge.connect({ launch: true });
    const [port] = t.ports;
    const hello = must(must(port).sent[0]);
    expect(hello.type).toBe('hello');
    expect(hello.payload).toMatchObject({ profileId: 'p1', launch: true });
    expect(t.bridge.state).toBe('connecting');
    must(port).receive(welcomeFor(hello));
    expect(t.bridge.state).toBe('connected');
    expect(t.bridge.welcome?.coreVersion).toBe('0.1.0');
    t.fireHeartbeat();
    expect(must(port).sent.at(-1)?.type).toBe('heartbeat');
    // Connecting again while connected does nothing.
    await t.bridge.connect({ launch: false });
    expect(t.ports.length).toBe(1);
  });

  it('reports the app as unavailable or outdated', async () => {
    const t = setup();
    await t.bridge.connect({ launch: false });
    const hello = must(must(t.ports[0]).sent[0]);
    must(t.ports[0]).receive(
      envelope(kinds.error, { code: 'APP_UNAVAILABLE', message: "Decay isn't running" }, hello.id),
    );
    expect(t.bridge.state).toBe('unavailable');
    expect(must(t.ports[0]).disconnected).toBe(true);

    await t.bridge.connect({ launch: false });
    const hello2 = must(must(t.ports[1]).sent[0]);
    must(t.ports[1]).receive(envelope(kinds.error, { code: 'VERSION_MISMATCH' }, hello2.id));
    expect(t.bridge.state).toBe('outdated');
  });

  it('gives up when nobody answers, and when the host goes away', async () => {
    const t = setup();
    await t.bridge.connect({ launch: false });
    t.fireTimers();
    expect(t.bridge.state).toBe('unavailable');
    expect(HANDSHAKE_TIMEOUT_MS).toBeGreaterThan(3_000); // longer than the host's app start

    await t.bridge.connect({ launch: false });
    must(t.ports[1]).receive(welcomeFor(must(must(t.ports[1]).sent[0])));
    must(t.ports[1]).drop();
    expect(t.bridge.state).toBe('unavailable');
    expect(t.bridge.send(envelope('heartbeat', {}))).toBe(false);
  });

  it('drops an attempt stuck before the handshake so the watchdog can reconnect', async () => {
    let now = 0;
    let release: (() => void) | undefined;
    const t = setup();
    const deps = t.deps;
    const realHello = deps.hello;
    deps.now = () => now;
    // The first hello() never settles (the worker was swapped mid-update).
    deps.hello = () =>
      new Promise((resolve) => {
        release = () => resolve(realHello());
      });
    void t.bridge.connect({ launch: false });
    expect(t.bridge.state).toBe('connecting');
    deps.hello = realHello;
    // A watchdog tick inside the window leaves the attempt alone…
    now = STUCK_ATTEMPT_MS - 1;
    await t.bridge.connect({ launch: false });
    expect(t.ports.length).toBe(0);
    // …a later one drops it and connects.
    now = STUCK_ATTEMPT_MS + 1;
    await t.bridge.connect({ launch: false });
    expect(t.ports.length).toBe(1);
    must(t.ports[0]).receive(welcomeFor(must(must(t.ports[0]).sent[0])));
    expect(t.bridge.state).toBe('connected');
    // The stale attempt finally settles: it must not open a second port.
    release?.();
    await new Promise((r) => setTimeout(r, 0));
    expect(t.ports.length).toBe(1);
    expect(t.bridge.state).toBe('connected');
  });

  it('opens one port for overlapping connects, and upgrades a quiet attempt to launch', async () => {
    const t = setup();
    // Browser start-up: the worker's quiet reconnect and onStartup's launch race.
    const quiet = t.bridge.connect({ launch: false });
    const launch = t.bridge.connect({ launch: true });
    await Promise.all([quiet, launch]);
    expect(t.ports.length).toBe(1);
    const first = must(must(t.ports[0]).sent[0]);
    expect(first.payload).toMatchObject({ launch: false });
    // The quiet attempt finds no app: it's retried once, now allowed to start the app.
    must(t.ports[0]).receive(envelope(kinds.error, { code: 'APP_UNAVAILABLE' }, first.id));
    await Promise.resolve();
    await Promise.resolve();
    expect(t.ports.length).toBe(2);
    expect(must(must(t.ports[1]).sent[0]).payload).toMatchObject({ launch: true });
  });

  it('stays off without the permission', async () => {
    const t = setup({ noPermission: true });
    await t.bridge.connect({ launch: true });
    expect(t.bridge.state).toBe('off');
  });

  it('matches replies to requests and passes other messages on', async () => {
    const t = setup();
    await t.bridge.connect({ launch: false });
    const port = must(t.ports[0]);
    port.receive(welcomeFor(must(port.sent[0])));
    const req = envelope(kinds.importArchive, { items: [], rules: [], settings: {} });
    const reply = t.bridge.request(req, 60_000);
    port.receive(envelope(kinds.probe, { tabIds: [1] }));
    port.receive(envelope(kinds.ack, { ok: true }, req.id));
    await expect(reply).resolves.toMatchObject({ type: 'ack', payload: { ok: true } });
    expect(t.messages.map((m) => m.type)).toEqual(['probe']);

    // A pending request fails when the link drops.
    const lost = t.bridge.request(envelope(kinds.importArchive, {}), 60_000);
    port.drop();
    await expect(lost).rejects.toThrow(/disconnected/);
  });

  it('chunks big messages and reassembles big replies', async () => {
    const t = setup();
    await t.bridge.connect({ launch: false });
    const port = must(t.ports[0]);
    port.receive(welcomeFor(must(port.sent[0])));
    const big = envelope(kinds.importArchive, {
      items: Array(30_000).fill({ title: 'x'.repeat(100) }),
    });
    expect(t.bridge.send(big)).toBe(true);
    const chunks = port.sent.filter((m) => m.chunk);
    expect(chunks.length).toBeGreaterThan(1);
    // The app's chunked messages come back whole.
    const { splitEnvelope } = await import('./chunk');
    for (const c of splitEnvelope(
      envelope(kinds.config, { probeEnabled: true, excludedDomains: Array(80_000).fill('a.com') }),
    )) {
      port.receive(c);
    }
    expect(t.messages.at(-1)?.type).toBe('config');
  });
});
