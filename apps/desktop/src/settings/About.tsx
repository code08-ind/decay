import { getVersion } from '@tauri-apps/api/app';
import { Leaf } from 'lucide-react';
import { useEffect, useState } from 'react';
import { Group, Row, TabHeader } from './parts';

// About (Designs/09h): version and what Decay promises.
export function About() {
  const [version, setVersion] = useState('');
  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch(() => {});
  }, []);
  return (
    <>
      <TabHeader title="About" />
      <div className="grid gap-4">
        <section className="flex items-center gap-4 rounded-card border border-line bg-surface p-5 shadow-sm">
          <span
            aria-hidden
            className="grid size-12 place-items-center rounded-xl bg-feature text-feature-ink"
          >
            <Leaf size={22} />
          </span>
          <div>
            <p className="font-serif text-2xl">Decay</p>
            <p className="text-sm text-muted">{version ? `Version ${version}` : '\u00a0'}</p>
          </div>
        </section>
        <Group title="Promises">
          <Row
            label="Nothing is lost"
            description="Every archived tab can be searched and restored."
          />
          <Row
            label="Local only"
            description="Your browsing history never leaves this device; the database is encrypted."
          />
          <Row label="Undo anything" description="Any recent sweep can be undone in one click." />
        </Group>
      </div>
    </>
  );
}
