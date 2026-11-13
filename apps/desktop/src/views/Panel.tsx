import { listen } from '@tauri-apps/api/event';
import { useEffect, useRef, useState } from 'react';
import { api } from '../api';

// Placeholder until the search panel (P1.9): opens with the hotkey, Esc hides it.
export function Panel() {
  const input = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState('');
  useEffect(() => {
    // The prefill waits in the app until the page takes it: on load (a cold start from
    // decay://search) and each time the panel opens.
    const take = () =>
      api
        .takePending()
        .then((p) => {
          setQuery(p.prefill ?? '');
          input.current?.focus();
          input.current?.select();
        })
        .catch(() => {});
    take();
    const off = listen('panel:open', take);
    return () => {
      off.then((f) => f());
    };
  }, []);
  return (
    <main className="panel">
      <input
        ref={input}
        aria-label="Search your archive"
        placeholder="Search your archive…"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Escape') api.hidePanel();
        }}
      />
      <p className="muted">Search arrives with the panel UI.</p>
    </main>
  );
}
