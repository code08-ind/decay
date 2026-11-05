// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Item } from '@/lib/db/types';
import type { ReportItem, ShadowReport } from '@/lib/shadow/report';
import { ShadowReview } from './ShadowReview';

afterEach(cleanup);

let n = 0;
const entry = (title: string, ageH: number, protectedBy?: 'pinned'): ReportItem => {
  const item = {
    id: `i${n++}`,
    kind: 'tab',
    appId: 'app',
    title,
    url: `https://${title.toLowerCase()}.com/x`,
    domain: `${title.toLowerCase()}.com`,
    pinned: !!protectedBy,
    state: 'stale',
    activeAgeS: ageH * 3600,
    firstSeenAt: 0,
    lastUsedAt: 0,
    stateChangedAt: 0,
    restoreCount: 0,
  } as Item;
  return protectedBy
    ? { item, decision: { action: 'protect', reason: { type: 'protected', by: protectedBy } } }
    : {
        item,
        decision: {
          action: 'archive',
          reason: { type: 'unused', activeHours: ageH, threshold: 60 },
        },
      };
};

const report = (over: Partial<ShadowReport> = {}): ShadowReport => ({
  mode: 'shadow',
  activeDays: 3,
  targetDays: 7,
  ready: false,
  todayCounted: false,
  dayNumber: 4,
  topDomains: [],
  flaggedTotal: 40,
  regretTotal: 2,
  regretRate: 0.05,
  wouldArchive: ['Stripe', 'Jira', 'Linear', 'Figma', 'Notion', 'Vercel'].map((t, i) =>
    entry(t, 70 - i),
  ),
  kept: [entry('Docs', 80, 'pinned')],
  keptByGroup: { pinned: 1, audio: 0, active: 0, form: 0, rules: 0, hold: 0, other: 0 },
  nextSimulationAt: 1_000_000 + 12 * 60_000,
  ...over,
});

const props = (over = {}) => ({
  report: report(),
  now: 1_000_000,
  liveTabs: 42,
  automaticAvailable: true,
  onTurnOnAutomatic: vi.fn(),
  onBackToShadow: vi.fn(),
  onKeepWatching: vi.fn(),
  onProtect: vi.fn(),
  ...over,
});

describe('ShadowReview', () => {
  it('summarises the shadow period', () => {
    render(<ShadowReview {...props()} />);
    expect(screen.getByRole('heading', { name: 'Decay is watching quietly' })).toBeTruthy();
    expect(screen.getByText(/Day 4 of 7 · Decay would have archived 40 tabs/)).toBeTruthy();
    expect(screen.getByText('5% regret')).toBeTruthy();
    expect(screen.getByText('Next simulation in 12m')).toBeTruthy();
    expect(screen.getByText('42 open tabs tracked')).toBeTruthy();
  });

  it('offers automatic mode when ready', () => {
    const p = props({ report: report({ ready: true, activeDays: 7 }) });
    render(<ShadowReview {...p} />);
    fireEvent.click(screen.getByRole('button', { name: /Keep watching/ }));
    expect(p.onKeepWatching).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole('button', { name: /Turn on automatic mode/ }));
    expect(p.onTurnOnAutomatic).not.toHaveBeenCalled(); // confirm step first
    fireEvent.click(screen.getByRole('button', { name: 'Turn on' }));
    expect(p.onTurnOnAutomatic).toHaveBeenCalledOnce();
  });

  it('asks for confirmation before ending shadow mode early', () => {
    const p = props();
    render(<ShadowReview {...p} />);
    fireEvent.click(screen.getByRole('button', { name: 'Turn on automatic mode early' }));
    expect(screen.getByText(/ends shadow mode before day 7/)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(p.onTurnOnAutomatic).not.toHaveBeenCalled();
  });

  it('does not offer automatic mode before sweeps exist', () => {
    render(
      <ShadowReview {...props({ automaticAvailable: false, report: report({ ready: true }) })} />,
    );
    expect(screen.queryByRole('button', { name: /Turn on automatic/ })).toBeNull();
    expect(screen.getByText(/Automatic mode arrives in the next update/)).toBeTruthy();
  });

  it('shows honest copy and a way back in paused and automatic modes', () => {
    const p = props({ report: report({ mode: 'paused' }) });
    const { unmount } = render(<ShadowReview {...p} />);
    expect(screen.getByRole('heading', { name: 'Decay is paused' })).toBeTruthy();
    expect(screen.getByText(/Nothing is simulated or archived until you resume/)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /Back to shadow mode/ }));
    expect(p.onBackToShadow).toHaveBeenCalledOnce();
    unmount();
    render(<ShadowReview {...props({ report: report({ mode: 'automatic' }) })} />);
    expect(screen.getByText(/During shadow mode, Decay would have archived 40 tabs/)).toBeTruthy();
  });

  it('implements the tabs pattern with arrow keys', () => {
    render(<ShadowReview {...props()} />);
    const archive = screen.getByRole('tab', { name: /Would archive/ });
    const panel = screen.getByRole('tabpanel');
    expect(archive.getAttribute('aria-controls')).toBe(panel.id);
    expect(panel.getAttribute('aria-labelledby')).toBe(archive.id);
    fireEvent.keyDown(archive, { key: 'ArrowRight' });
    const kept = screen.getByRole('tab', { name: /Kept/ });
    expect(kept.getAttribute('aria-selected')).toBe('true');
    expect(kept.getAttribute('tabindex')).toBe('0');
    expect(archive.getAttribute('tabindex')).toBe('-1');
    expect(document.activeElement).toBe(kept);
    // The list keeps its list semantics inside the panel.
    expect(screen.getByRole('tabpanel').querySelector('ul')?.getAttribute('role')).toBeNull();
  });

  it('shows 5 rows, expands, filters and protects', () => {
    const p = props();
    render(<ShadowReview {...p} />);
    expect(
      screen.getAllByRole('listitem').filter((li) => li.closest('[role=tabpanel]')),
    ).toHaveLength(5);
    fireEvent.click(screen.getByRole('button', { name: 'View all 6' }));
    expect(screen.getByText('Vercel')).toBeTruthy();
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'jira' } });
    expect(screen.queryByText('Stripe')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Always keep site: jira.com' }));
    expect(p.onProtect).toHaveBeenCalledWith(
      expect.objectContaining({ item: expect.objectContaining({ title: 'Jira' }) }),
    );
  });

  it('lists kept tabs with their reason', () => {
    render(<ShadowReview {...props()} />);
    fireEvent.click(screen.getByRole('tab', { name: 'Kept (1)' }));
    expect(screen.getByText(/Protected: pinned/)).toBeTruthy();
  });

  it('has a friendly empty state', () => {
    render(
      <ShadowReview
        {...props({ report: report({ wouldArchive: [], flaggedTotal: 0, regretRate: null }) })}
      />,
    );
    expect(screen.getByText(/Nothing would be archived right now/)).toBeTruthy();
    expect(screen.getByText('— regret')).toBeTruthy();
  });
});
