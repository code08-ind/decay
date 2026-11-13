import { SearchInput, Shortcut } from '@decay/ui';
import { listen } from '@tauri-apps/api/event';
import { useEffect, useRef, useState } from 'react';
import { api } from '../api';

// Placeholder until the search panel (P1.9): opens with the hotkey; Esc clears, then hides.
export function Panel() {
  const box = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState('');
  useEffect(() => {
    // The prefill waits in the app until the page takes it: on load (a cold start from
    // decay://search) and each time the panel opens.
    const take = () =>
      api
        .takePending()
        .then((p) => {
          setQuery(p.prefill ?? '');
          const input = box.current?.querySelector('input');
          input?.focus();
          input?.select();
        })
        .catch(() => {});
    take();
    const off = listen('panel:open', take);
    return () => {
      off.then((f) => f());
    };
  }, []);
  return (
    <main
      ref={box}
      className="grid h-screen content-start gap-3 rounded-card border border-line bg-surface p-4"
    >
      <h1 className="sr-only">Search your archive</h1>
      <SearchInput
        large
        label="Search your archive"
        placeholder="Search your archive…"
        value={query}
        onValueChange={setQuery}
        onEscape={() => api.hidePanel()}
      />
      <p className="flex items-center gap-2 px-1 text-xs text-muted">
        Search arrives with the panel UI. <Shortcut accelerator="Escape" /> closes.
      </p>
    </main>
  );
}
