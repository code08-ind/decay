// Test-only: jsdom internals used to fire *trusted* events (lib/probe/inspect.test.ts).
declare module 'jsdom/lib/jsdom/living/helpers/events.js' {
  export function fireAnEvent(
    type: string,
    target: unknown,
    eventInterface?: unknown,
    attributes?: Record<string, unknown>,
  ): boolean;
}
declare module 'jsdom/lib/generated/idl/utils.js' {
  export function implForWrapper(wrapper: unknown): unknown;
}
