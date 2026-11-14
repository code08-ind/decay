import { listen } from '@tauri-apps/api/event';
import { useEffect, useState } from 'react';
import { api, type EngineStatus } from './api';

/** The engine's status, kept current by `status:changed`. */
export function useStatus(): EngineStatus | null {
  const [status, setStatus] = useState<EngineStatus | null>(null);
  useEffect(() => {
    let live = true;
    const off = listen<EngineStatus>('status:changed', (e) => setStatus(e.payload));
    api
      .getStatus()
      .then((s) => live && setStatus(s))
      .catch(() => {});
    return () => {
      live = false;
      off.then((f) => f());
    };
  }, []);
  return status;
}

/** The route from the URL hash, updated by `navigate` (deep links, tray) and by `go`. */
export function useRoute(): [string, (route: string) => void] {
  const [route, setRoute] = useState(() => location.hash.slice(1) || '/');
  useEffect(() => {
    const off = listen<{ route: string }>('navigate', (e) => setRoute(e.payload.route));
    return () => {
      off.then((f) => f());
    };
  }, []);
  return [route, setRoute];
}
