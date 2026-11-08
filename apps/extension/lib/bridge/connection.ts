// The native connection to the Decay app (Software Design §4.7 `background/bridge.ts`):
// `connectNative('com.decay.host')`, the hello → welcome handshake, a heartbeat while
// connected, chunking both ways, and request / reply matching.
//
// Reconnecting is the caller's job (a `chrome.alarms` watchdog): the bridge only reports
// its state. `launch: true` in `hello` lets the host start the app; the caller sets it only
// at browser start-up or on a user action, so a reconnect never reopens an app the user
// quit.

import { Reassembler, SplitError, splitEnvelope } from './chunk';
import {
  type Envelope,
  type ErrorPayload,
  envelope,
  type Hello,
  HOST_NAME,
  isEnvelope,
  kinds,
  type Welcome,
} from './protocol';

/** What the app link is doing. */
export type BridgeState =
  /** Not connected (never tried, or no permission). */
  | 'off'
  | 'connecting'
  | 'connected'
  /** The app isn't running or the host isn't installed. */
  | 'unavailable'
  /** The app needs a newer extension (VERSION_MISMATCH). */
  | 'outdated';

/** The part of `chrome.runtime.Port` the bridge uses. */
export interface PortLike {
  postMessage(message: unknown): void;
  disconnect(): void;
  onMessage: { addListener(fn: (message: unknown) => void): void };
  onDisconnect: { addListener(fn: () => void): void };
}

export interface BridgeDeps {
  connectNative(name: string): PortLike;
  /** The `hello` payload (minus `launch`). */
  hello(): Promise<Omit<Hello, 'launch'>>;
  /** Messages from the app that aren't replies to a pending request. */
  onMessage(env: Envelope): void;
  onState(state: BridgeState, detail?: string): void;
  now(): number;
  setTimer(fn: () => void, ms: number): unknown;
  clearTimer(handle: unknown): void;
  setRepeating(fn: () => void, ms: number): unknown;
  clearRepeating(handle: unknown): void;
}

/** No `welcome` within this long: the app isn't answering. */
export const HANDSHAKE_TIMEOUT_MS = 10_000;

interface Pending {
  resolve(env: Envelope): void;
  reject(error: Error): void;
  timer: unknown;
}

export class Bridge {
  state: BridgeState = 'off';
  welcome: Welcome | null = null;
  private port: PortLike | null = null;
  private reassembler = new Reassembler();
  private helloId: string | null = null;
  private heartbeat: unknown = null;
  private handshakeTimer: unknown = null;
  private pending = new Map<string, Pending>();
  /** The current attempt may start the app. */
  private launching = false;
  /** A launching attempt was asked for while a quiet one was running. */
  private launchAfter = false;

  constructor(private readonly deps: BridgeDeps) {}

  private setState(state: BridgeState, detail?: string) {
    this.state = state;
    this.deps.onState(state, detail);
  }

  /** Connects unless already connecting or connected. */
  async connect(opts: { launch: boolean }): Promise<void> {
    if (this.state === 'connected') return;
    if (this.state === 'connecting') {
      // A user action (or browser start-up) during a quiet reconnect: if that attempt
      // finds no app, try once more and let the host start it.
      if (opts.launch && !this.launching) this.launchAfter = true;
      return;
    }
    // Claimed before any await, so two calls can't both open a port.
    this.setState('connecting');
    this.launching = opts.launch;
    this.launchAfter = false;
    let hello: Omit<Hello, 'launch'>;
    let port: PortLike;
    try {
      hello = await this.deps.hello();
      port = this.deps.connectNative(HOST_NAME);
    } catch (e) {
      // No `nativeMessaging` permission, or the API is missing.
      this.setState('off', e instanceof Error ? e.message : String(e));
      return;
    }
    this.port = port;
    this.reassembler = new Reassembler();
    port.onMessage.addListener((m) => this.received(port, m));
    port.onDisconnect.addListener(() => this.closed(port));
    const env = envelope(kinds.hello, { ...hello, launch: opts.launch });
    this.helloId = env.id;
    // The host may need ~3 s to start the app; then the app answers.
    this.handshakeTimer = this.deps.setTimer(() => {
      if (this.port === port && this.state === 'connecting')
        this.end('unavailable', 'no answer from Decay');
    }, HANDSHAKE_TIMEOUT_MS);
    if (!this.post(port, env)) this.end('unavailable', 'the connection closed');
  }

  /** Posts to a port that may have just closed (that throws). */
  private post(port: PortLike, message: unknown): boolean {
    try {
      port.postMessage(message);
      return true;
    } catch {
      return false;
    }
  }

  /** Disconnects on purpose (the user turned the link off). */
  disconnect(): void {
    this.launchAfter = false;
    this.end('off');
  }

  /** Sends a message (chunked if needed). False when not connected or too big. */
  send(env: Envelope): boolean {
    const port = this.port;
    if (!port || this.state !== 'connected') return false;
    let frames: Envelope[];
    try {
      frames = splitEnvelope(env);
    } catch (e) {
      if (e instanceof SplitError) return false;
      throw e;
    }
    for (const frame of frames) {
      if (!this.post(port, frame)) return false;
    }
    return true;
  }

  /** Sends a message and waits for the reply (`replyTo` = its id). */
  request(env: Envelope, timeoutMs: number): Promise<Envelope> {
    return new Promise((resolve, reject) => {
      const timer = this.deps.setTimer(() => {
        this.pending.delete(env.id);
        reject(new Error('timed out'));
      }, timeoutMs);
      this.pending.set(env.id, { resolve, reject, timer });
      if (!this.send(env)) {
        this.deps.clearTimer(timer);
        this.pending.delete(env.id);
        reject(new Error('not connected'));
      }
    });
  }

  private received(port: PortLike, message: unknown) {
    if (port !== this.port || !isEnvelope(message)) return;
    let env: Envelope | null;
    try {
      env = this.reassembler.push(message, this.deps.now());
    } catch {
      return; // a broken chunk group is dropped
    }
    if (!env) return;
    if (this.state === 'connecting') {
      this.handshake(env);
      return;
    }
    const waiting = env.replyTo ? this.pending.get(env.replyTo) : undefined;
    if (waiting && env.replyTo) {
      this.deps.clearTimer(waiting.timer);
      this.pending.delete(env.replyTo);
      waiting.resolve(env);
      return;
    }
    if (env.type === kinds.error) {
      const code = (env.payload as ErrorPayload | null)?.code;
      if (code === 'APP_UNAVAILABLE') {
        this.end('unavailable', 'Decay closed');
        return;
      }
    }
    this.deps.onMessage(env);
  }

  private handshake(env: Envelope) {
    if (env.type === kinds.welcome && env.replyTo === this.helloId) {
      this.deps.clearTimer(this.handshakeTimer);
      this.welcome = env.payload as Welcome;
      this.setState('connected');
      const sec = Math.max(5, Number(this.welcome?.config?.heartbeatSec) || 20);
      this.heartbeat = this.deps.setRepeating(() => {
        this.send(envelope(kinds.heartbeat, { at: this.deps.now() }));
      }, sec * 1000);
      return;
    }
    if (env.type === kinds.error) {
      const error = env.payload as ErrorPayload | null;
      const outdated = error?.code === 'VERSION_MISMATCH';
      this.end(outdated ? 'outdated' : 'unavailable', error?.message);
    }
  }

  private closed(port: PortLike) {
    if (port !== this.port) return;
    // `chrome.runtime.lastError` explains why (host not installed, app gone…).
    const lastError = (
      globalThis as { chrome?: { runtime?: { lastError?: { message?: string } } } }
    ).chrome?.runtime?.lastError?.message;
    this.end(this.state === 'outdated' ? 'outdated' : 'unavailable', lastError, false);
  }

  private end(state: BridgeState, detail?: string, disconnect = true) {
    const port = this.port;
    this.port = null;
    this.helloId = null;
    this.welcome = null;
    this.deps.clearTimer(this.handshakeTimer);
    if (this.heartbeat !== null) this.deps.clearRepeating(this.heartbeat);
    this.heartbeat = null;
    for (const [id, p] of this.pending) {
      this.deps.clearTimer(p.timer);
      p.reject(new Error('disconnected'));
      this.pending.delete(id);
    }
    if (disconnect && port) {
      try {
        port.disconnect();
      } catch {
        // already closed
      }
    }
    this.setState(state, detail);
    if (state === 'unavailable' && this.launchAfter) {
      this.launchAfter = false;
      void this.connect({ launch: true });
    }
  }
}
