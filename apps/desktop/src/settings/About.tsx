import { Button, Shortcut } from '@decay/ui';
import { getVersion } from '@tauri-apps/api/app';
import {
  ChevronRight,
  CircleHelp,
  ExternalLink,
  FileText,
  Keyboard,
  Leaf,
  MessageSquare,
  Sparkles,
} from 'lucide-react';
import { type ReactNode, useEffect, useState } from 'react';
import { api } from '../api';
import { Group, Row, TabHeader } from './parts';

// About (Designs/09h, PRD F32): version, help, the changelog, feedback (a prefilled GitHub
// issue with the version and OS only) and what Decay promises.
export function About(props: { go: (tab: string) => void; fail: (e: unknown) => void }) {
  const { go, fail } = props;
  const [version, setVersion] = useState('');
  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch(() => {});
  }, []);
  const open = (link: 'help' | 'changelog' | 'feedback') => api.openLink(link).catch(fail);
  return (
    <>
      <TabHeader title="About">Help, feedback and what’s changed.</TabHeader>
      <div className="grid gap-4">
        <section className="grid justify-items-center gap-3 rounded-card border border-line bg-surface p-8 text-center shadow-sm">
          <span
            aria-hidden
            className="grid size-20 place-items-center rounded-3xl bg-chip text-accent"
          >
            <Leaf size={40} />
          </span>
          <p className="font-serif text-5xl">Decay</p>
          <p className="rounded-full bg-surface-2 px-3 py-0.5 font-mono text-sm text-muted">
            {version ? `Version ${version}` : ' '}
          </p>
          <div className="mt-3 flex flex-wrap justify-center gap-2">
            <Button icon={<CircleHelp size={14} />} onClick={() => open('help')}>
              Help
            </Button>
            <Button icon={<FileText size={14} />} onClick={() => open('changelog')}>
              Changelog
            </Button>
            <Button icon={<MessageSquare size={14} />} onClick={() => open('feedback')}>
              Send feedback
            </Button>
          </div>
          <p className="max-w-md text-xs text-muted">
            Feedback opens a new GitHub issue with your Decay version and system filled in — nothing
            from your archive. You can edit everything before sending.
          </p>
        </section>
        <nav
          aria-label="Help"
          className="grid divide-y divide-line rounded-card border border-line bg-surface px-5 shadow-sm"
        >
          <LinkRow
            icon={<CircleHelp size={18} />}
            label="Help & guides"
            external
            onClick={() => open('help')}
          />
          <LinkRow
            icon={<Keyboard size={18} />}
            label="Keyboard shortcuts"
            shortcut="CmdOrCtrl+/"
            onClick={() => go('shortcuts')}
          />
          <LinkRow
            icon={<Sparkles size={18} />}
            label="What’s new in this version"
            onClick={() => api.showWhatsNew().catch(fail)}
          />
        </nav>
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

function LinkRow(props: {
  icon: ReactNode;
  label: string;
  shortcut?: string;
  external?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={props.onClick}
      className="flex items-center gap-4 py-3 text-left text-sm font-medium hover:text-accent"
    >
      <span
        aria-hidden
        className="grid size-10 place-items-center rounded-full bg-surface-2 text-muted"
      >
        {props.icon}
      </span>
      <span className="flex flex-1 items-center gap-2">
        {props.label}
        {props.shortcut && <Shortcut accelerator={props.shortcut} />}
        {props.external && <span className="sr-only">(opens in your browser)</span>}
      </span>
      {props.external ? (
        <ExternalLink size={16} className="text-muted" aria-hidden />
      ) : (
        <ChevronRight size={16} className="text-muted" aria-hidden />
      )}
    </button>
  );
}
