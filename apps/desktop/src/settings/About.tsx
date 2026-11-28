import { Button, Shortcut } from '@decay/ui';
import { getVersion } from '@tauri-apps/api/app';
import {
  ChevronRight,
  CircleHelp,
  ExternalLink,
  FileText,
  Keyboard,
  MessageSquare,
  Sparkles,
} from 'lucide-react';
import { type ReactNode, useEffect, useState } from 'react';
import { api } from '../api';
import { DecayLeaf } from '../DecayLeaf';
import { t } from '../strings';
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
      <TabHeader title={t('settings.tab.about')}>{t('about.subtitle')}</TabHeader>
      <div className="grid gap-4">
        <section className="grid justify-items-center gap-3 rounded-card border border-line bg-surface p-8 text-center shadow-sm">
          <span
            aria-hidden
            className="grid size-20 place-items-center rounded-3xl bg-chip text-accent"
          >
            <DecayLeaf size={40} />
          </span>
          <p className="font-serif text-5xl">Decay</p>
          <p className="rounded-full bg-surface-2 px-3 py-0.5 font-mono text-sm text-muted">
            {version ? t('about.version', { version }) : ' '}
          </p>
          <div className="mt-3 flex flex-wrap justify-center gap-2">
            <Button icon={<CircleHelp size={14} />} onClick={() => open('help')}>
              {t('about.help')}
            </Button>
            <Button icon={<FileText size={14} />} onClick={() => open('changelog')}>
              {t('about.changelog')}
            </Button>
            <Button icon={<MessageSquare size={14} />} onClick={() => open('feedback')}>
              {t('about.feedback')}
            </Button>
          </div>
          <p className="max-w-md text-xs text-muted">{t('about.feedbackNote')}</p>
        </section>
        <nav
          aria-label={t('about.links')}
          className="grid divide-y divide-line rounded-card border border-line bg-surface px-5 shadow-sm"
        >
          <LinkRow
            icon={<CircleHelp size={18} />}
            label={t('about.guides')}
            external
            onClick={() => open('help')}
          />
          <LinkRow
            icon={<Keyboard size={18} />}
            label={t('about.shortcuts')}
            shortcut="CmdOrCtrl+/"
            onClick={() => go('shortcuts')}
          />
          <LinkRow
            icon={<Sparkles size={18} />}
            label={t('about.whatsNew')}
            onClick={() => api.showWhatsNew().catch(fail)}
          />
        </nav>
        <Group title={t('about.promises')}>
          <Row label={t('about.promise.lost')} description={t('about.promise.lost.text')} />
          <Row label={t('about.promise.local')} description={t('about.promise.local.text')} />
          <Row label={t('about.promise.undo')} description={t('about.promise.undo.text')} />
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
      className="flex items-center gap-4 py-3 text-start text-sm font-medium hover:text-accent"
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
        {props.external && <span className="sr-only">{t('about.opensInBrowser')}</span>}
      </span>
      {props.external ? (
        <ExternalLink size={16} className="rtl:-scale-x-100 text-muted" aria-hidden />
      ) : (
        <ChevronRight size={16} className="rtl:-scale-x-100 text-muted" aria-hidden />
      )}
    </button>
  );
}
