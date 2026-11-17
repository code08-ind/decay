// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { InsightsPage } from '@/entrypoints/insights/InsightsPage';
import { db } from '@/lib/db/db';
import { buildInsights, dayRange } from '@/lib/metrics/insights';
import { rules } from '@/lib/rules/rules';
import { emptyMetrics } from '@/lib/shadow/shadow';
import { type InsightsProps, InsightsView } from './Insights';

afterEach(cleanup);

const NOW = new Date(2026, 10, 7, 15, 0).getTime();
const days = dayRange(NOW, 30);
const metrics = days.map((day, i) => ({
  ...emptyMetrics(day),
  openTabsP50: 60 - i,
  openTabsSamples: [60 - i],
  activeSeconds: 7200,
  archivedCount: i === 29 ? 3 : 0,
}));
const insights = buildInsights({
  now: NOW,
  range: 30,
  mode: 'automatic',
  metrics,
  items: [{ id: 'x', domain: 'docs.example', archivedAt: NOW - 1000, restoreCount: 1 } as never],
});

function renderProps(over: Partial<InsightsProps> = {}): InsightsProps {
  return {
    insights,
    mode: 'automatic',
    onRangeChange: vi.fn(),
    onExportCsv: vi.fn(),
    sharePreview: null,
    onShare: vi.fn(),
    onShareDownload: vi.fn(),
    onShareCancel: vi.fn(),
    ...over,
  };
}

function renderView(over: Partial<InsightsProps> = {}) {
  const props: InsightsProps = {
    insights,
    mode: 'automatic',
    onRangeChange: vi.fn(),
    onExportCsv: vi.fn(),
    sharePreview: null,
    onShare: vi.fn(),
    onShareDownload: vi.fn(),
    onShareCancel: vi.fn(),
    ...over,
  };
  render(<InsightsView {...props} />);
  return props;
}

describe('Insights', () => {
  it('shows the summary tiles, goals and site lists', () => {
    renderView();
    const summary = screen.getByRole('region', { name: 'Summary' });
    expect(within(summary).getByText('Median open tabs')).toBeTruthy();
    expect(within(summary).getByText(/in week 1/)).toBeTruthy();
    expect(within(summary).getByText('3')).toBeTruthy(); // archived
    expect(screen.getAllByText('docs.example').length).toBeGreaterThan(0);
    expect(screen.getByRole('heading', { name: 'How Decay is doing' })).toBeTruthy();
    expect(screen.getByText('Search to restore (14 days)')).toBeTruthy();
    expect(screen.getAllByText('No data yet').length).toBeGreaterThan(0);
  });

  it('has a hover/focus readout per day and a table view', () => {
    renderView();
    const bars = screen.getAllByRole('button', { name: /open tabs?$|no data$/ });
    expect(bars).toHaveLength(30);
    fireEvent.focus(bars[29] as HTMLElement);
    expect(screen.getByText('31 tabs')).toBeTruthy();
    fireEvent.click(screen.getByText('Show as table'));
    expect(screen.getAllByRole('row')).toHaveLength(31);
  });

  it('changes the range and exports', () => {
    const props = renderView();
    fireEvent.click(screen.getByLabelText('Last 7 days'));
    expect(props.onRangeChange).toHaveBeenCalledWith(7);
    fireEvent.click(screen.getByRole('button', { name: /Export CSV/ }));
    expect(props.onExportCsv).toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: /Share with beta team/ }));
    expect(props.onShare).toHaveBeenCalled();
  });

  it('previews the exact file before saving it', () => {
    const props = renderView({ sharePreview: '{\n  "format": "decay-beta-metrics"\n}' });
    const dialog = screen.getByRole('dialog', { name: 'Share with the beta team' });
    expect(within(dialog).getByText(/decay-beta-metrics/)).toBeTruthy();
    expect(document.activeElement?.textContent).toContain('Save file');
    fireEvent.click(within(dialog).getByRole('button', { name: /Save file/ }));
    expect(props.onShareDownload).toHaveBeenCalled();
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(props.onShareCancel).toHaveBeenCalled();
  });

  it('keeps the chart keyboard-reachable after the range shrinks', () => {
    const { rerender } = render(<InsightsView {...renderProps()} insights={insights} />);
    const seven = buildInsights({ now: NOW, range: 7, mode: 'automatic', metrics, items: [] });
    rerender(<InsightsView {...renderProps()} insights={seven} />);
    const bars = screen.getAllByRole('button', { name: /open tabs?$|no data$/ });
    expect(bars).toHaveLength(7);
    expect(bars.filter((b) => b.tabIndex === 0)).toHaveLength(1);
  });

  it('says "1 sweep undone", links the other pages, and marks this one', () => {
    const one = buildInsights({
      now: NOW,
      range: 30,
      mode: 'automatic',
      metrics: [{ ...emptyMetrics(days[29] as string), undoCount: 1 }],
      items: [],
    });
    renderView({ insights: one });
    expect(screen.getByText('1 sweep undone')).toBeTruthy();
    const nav = screen.getByRole('navigation', { name: 'Decay pages' });
    expect(within(nav).getByRole('link', { name: 'Sweeps' }).getAttribute('href')).toBe(
      'sweeps.html',
    );
    expect(within(nav).getByRole('link', { name: 'Insights' }).getAttribute('aria-current')).toBe(
      'page',
    );
  });

  it('keeps Tab inside the share dialog and says what the file holds', () => {
    renderView({ sharePreview: '{}' });
    const dialog = screen.getByRole('dialog');
    expect(within(dialog).getByText(/install id is random/)).toBeTruthy();
    const save = within(dialog).getByRole('button', { name: /Save file/ });
    const close = within(dialog).getByRole('button', { name: 'Close' });
    expect(document.activeElement).toBe(save);
    fireEvent.keyDown(window, { key: 'Tab' });
    expect(document.activeElement).toBe(close);
    fireEvent.keyDown(window, { key: 'Tab', shiftKey: true });
    expect(document.activeElement).toBe(save);
  });

  it('explains an empty chart', () => {
    renderView({
      insights: buildInsights({ now: NOW, range: 7, mode: 'shadow', metrics: [], items: [] }),
    });
    expect(screen.getByText(/No samples yet/)).toBeTruthy();
    expect(screen.queryByText('Show as table')).toBeNull();
  });

  it('has no "Always keep" button without onProtectSite', () => {
    renderView();
    expect(screen.queryByRole('button', { name: /Always keep/ })).toBeNull();
  });

  it('"Always keep" on a restored-most row: busy, then kept', async () => {
    let resolve = () => {};
    const onProtectSite = vi.fn(
      () =>
        new Promise<void>((r) => {
          resolve = r;
        }),
    );
    renderView({ onProtectSite });
    const button = screen.getByRole('button', { name: 'Always keep docs.example' });
    fireEvent.click(button);
    expect(onProtectSite).toHaveBeenCalledWith('docs.example');
    expect(button.getAttribute('aria-busy')).toBe('true');
    expect((button as HTMLButtonElement).disabled).toBe(true);
    expect(button.textContent).toContain('Keeping…');
    await act(async () => resolve());
    expect(screen.queryByRole('button', { name: 'Always keep docs.example' })).toBeNull();
    expect(screen.getByText('Always kept')).toBeTruthy();
  });

  it('shows sites already kept, and keeps the button after a failure', async () => {
    const { rerender } = render(
      <InsightsView
        {...renderProps({ onProtectSite: vi.fn(), protectedSites: new Set(['docs.example']) })}
      />,
    );
    expect(screen.queryByRole('button', { name: /Always keep/ })).toBeNull();
    expect(screen.getByText('Always kept')).toBeTruthy();
    const onProtectSite = vi.fn().mockRejectedValue(new Error('nope'));
    rerender(<InsightsView {...renderProps({ onProtectSite, error: 'Could not' })} />);
    fireEvent.click(screen.getByRole('button', { name: 'Always keep docs.example' }));
    await act(async () => {});
    const button = screen.getByRole('button', { name: 'Always keep docs.example' });
    expect((button as HTMLButtonElement).disabled).toBe(false);
    expect(screen.getByRole('alert').textContent).toBe('Could not');
  });
});

describe('Insights page: "Always keep"', () => {
  afterEach(async () => {
    vi.restoreAllMocks();
    await Promise.all([db.items.clear(), db.rules.clear()]);
  });

  const seed = () =>
    db.items.bulkAdd([
      { id: 'a', domain: 'docs.example', restoreCount: 3 },
      { id: 'b', domain: 'mail.example', restoreCount: 2 },
    ] as never);

  it('makes a protect rule for the site', async () => {
    await seed();
    render(<InsightsPage />);
    fireEvent.click(await screen.findByRole('button', { name: 'Always keep docs.example' }));
    await screen.findByText('Always kept');
    const saved = await db.rules.where('match').equals('docs.example').toArray();
    expect(saved.map((r) => [r.scope, r.action, r.enabled])).toEqual([['domain', 'protect', true]]);
  });

  it('turns on a rule for the site that was turned off', async () => {
    await seed();
    await rules.create({
      scope: 'domain',
      match: 'docs.example',
      action: 'protect',
      enabled: false,
    });
    render(<InsightsPage />);
    fireEvent.click(await screen.findByRole('button', { name: 'Always keep docs.example' }));
    await screen.findByText('Always kept');
    const saved = await db.rules.where('match').equals('docs.example').toArray();
    expect(saved.map((r) => r.enabled)).toEqual([true]);
  });

  it('shows a site already kept, and shows errors', async () => {
    await seed();
    await rules.create({ scope: 'domain', match: 'docs.example', action: 'protect' });
    render(<InsightsPage />);
    await screen.findByText('Always kept');
    expect(screen.queryByRole('button', { name: 'Always keep docs.example' })).toBeNull();

    vi.spyOn(rules, 'alwaysKeep').mockRejectedValueOnce(new Error('disk full'));
    fireEvent.click(screen.getByRole('button', { name: 'Always keep mail.example' }));
    expect((await screen.findByRole('alert')).textContent).toBe(
      "Couldn't always keep mail.example: disk full",
    );
    await waitFor(() =>
      expect(
        (screen.getByRole('button', { name: 'Always keep mail.example' }) as HTMLButtonElement)
          .disabled,
      ).toBe(false),
    );
  });
});
