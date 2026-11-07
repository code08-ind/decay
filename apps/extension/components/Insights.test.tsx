// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { buildInsights, dayRange } from '@/lib/metrics/insights';
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
    const bars = screen.getAllByRole('button', { name: /open tabs$|no data$/ });
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
    const bars = screen.getAllByRole('button', { name: /open tabs$|no data$/ });
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
});
