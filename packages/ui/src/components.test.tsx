import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  AgeDot,
  Banner,
  Button,
  Chip,
  DialogActions,
  EmptyState,
  IconButton,
  LiveRegion,
  SearchInput,
  SegmentedControl,
  Shortcut,
  StatusDot,
  Switch,
  Tabs,
  Toast,
  tabId,
} from './index';

afterEach(cleanup);

function Presets() {
  const [v, setV] = useState<'gentle' | 'balanced' | 'aggressive'>('balanced');
  return (
    <SegmentedControl
      label="Pace"
      value={v}
      onChange={setV}
      options={[
        { value: 'gentle', label: 'Gentle' },
        { value: 'balanced', label: 'Balanced' },
        { value: 'aggressive', label: 'Aggressive' },
      ]}
    />
  );
}

function PricingField({ onEscape }: { onEscape: () => void }) {
  const [q, setQ] = useState('pricing');
  return <SearchInput label="Search" value={q} onValueChange={setQ} onEscape={onEscape} large />;
}

function EmptyField() {
  const [q, setQ] = useState('');
  return <SearchInput label="Search" value={q} onValueChange={setQ} />;
}

describe('DialogActions', () => {
  const order = (platform: 'mac' | 'windows') => {
    render(
      <DialogActions
        platform={platform}
        action={<Button variant="primary">Save</Button>}
        cancel={<Button>Cancel</Button>}
      />,
    );
    const names = screen.getAllByRole('button').map((b) => b.textContent);
    cleanup();
    return names;
  };

  it('puts the action on the right on macOS and first on Windows', () => {
    expect(order('mac')).toEqual(['Cancel', 'Save']);
    expect(order('windows')).toEqual(['Save', 'Cancel']);
  });
});

describe('Button', () => {
  it('is busy and disabled while loading, and announces its shortcut', () => {
    const onClick = vi.fn();
    render(
      <Button variant="primary" loading shortcut="⌘↵" onClick={onClick}>
        Restore
      </Button>,
    );
    const b = screen.getByRole('button', { name: 'Restore' });
    expect(b).toHaveProperty('disabled', true);
    expect(b.getAttribute('aria-busy')).toBe('true');
    expect(b.getAttribute('aria-keyshortcuts')).toBe('⌘↵');
    expect(b.getAttribute('type')).toBe('button');
  });

  it('icon buttons are named by their label', () => {
    render(<IconButton label="Settings" icon={<svg />} />);
    expect(screen.getByRole('button', { name: 'Settings' })).toBeTruthy();
  });
});

describe('SegmentedControl', () => {
  it('is a radio group with one tab stop, moved by arrow keys', () => {
    render(<Presets />);
    expect(screen.getByRole('radiogroup', { name: 'Pace' })).toBeTruthy();
    const balanced = screen.getByRole('radio', { name: 'Balanced' });
    expect(balanced.getAttribute('aria-checked')).toBe('true');
    expect(screen.getByRole('radio', { name: 'Gentle' }).tabIndex).toBe(-1);
    fireEvent.keyDown(balanced, { key: 'ArrowRight' });
    const aggressive = screen.getByRole('radio', { name: 'Aggressive' });
    expect(aggressive.getAttribute('aria-checked')).toBe('true');
    expect(document.activeElement).toBe(aggressive);
    fireEvent.keyDown(aggressive, { key: 'ArrowRight' }); // wraps
    expect(screen.getByRole('radio', { name: 'Gentle' }).getAttribute('aria-checked')).toBe('true');
    fireEvent.keyDown(screen.getByRole('radio', { name: 'Gentle' }), { key: 'End' });
    expect(aggressive.getAttribute('aria-checked')).toBe('true');
    fireEvent.click(screen.getByRole('radio', { name: 'Balanced' }));
    expect(balanced.getAttribute('aria-checked')).toBe('true');
  });
});

describe('Tabs', () => {
  it('selects with arrows and links tabs to panels', () => {
    const onSelect = vi.fn();
    render(
      <Tabs
        label="Library"
        current="archive"
        onSelect={onSelect}
        panelId={(id) => `panel-${id}`}
        tabs={[
          { id: 'archive', label: 'Archive', count: 12 },
          { id: 'live', label: 'Live now' },
          { id: 'sweeps', label: 'Sweeps' },
        ]}
      />,
    );
    const archive = screen.getByRole('tab', { name: /Archive/ });
    expect(archive.getAttribute('aria-selected')).toBe('true');
    expect(archive.getAttribute('aria-controls')).toBe('panel-archive');
    // Unselected tabs don't point at panels that aren't rendered.
    expect(screen.getByRole('tab', { name: 'Sweeps' }).getAttribute('aria-controls')).toBeNull();
    expect(archive.textContent).toContain('12');
    fireEvent.keyDown(archive, { key: 'ArrowLeft' });
    expect(onSelect).toHaveBeenLastCalledWith('sweeps');
    fireEvent.keyDown(archive, { key: 'Home' });
    expect(onSelect).toHaveBeenLastCalledWith('archive');
    fireEvent.keyDown(archive, { key: 'a' });
    expect(onSelect).toHaveBeenCalledTimes(2);
  });
});

it('two tablists never share tab ids', () => {
  const tabs = [{ id: 'a', label: 'A' }];
  render(
    <>
      <Tabs label="One" current="a" onSelect={() => {}} tabs={tabs} />
      <Tabs label="Two" current="a" onSelect={() => {}} tabs={tabs} idPrefix="two" />
    </>,
  );
  const [one, two] = screen.getAllByRole('tab');
  expect(one?.id).not.toBe(two?.id);
  expect(two?.id).toBe(tabId('two', 'a'));
});

describe('Switch', () => {
  it('toggles and is labelled and described', () => {
    const onChange = vi.fn();
    render(
      <Switch
        checked={false}
        onChange={onChange}
        label="Launch at login"
        description="Start with your Mac"
      />,
    );
    const s = screen.getByRole('switch', { name: 'Launch at login' });
    expect(s.getAttribute('aria-checked')).toBe('false');
    expect(document.getElementById(s.getAttribute('aria-describedby') ?? '')?.textContent).toBe(
      'Start with your Mac',
    );
    fireEvent.click(s);
    expect(onChange).toHaveBeenCalledWith(true);
  });
});

describe('SearchInput', () => {
  it('Esc clears first, then escapes', () => {
    const onEscape = vi.fn();
    render(<PricingField onEscape={onEscape} />);
    const input = screen.getByRole('searchbox', { name: 'Search' }) as HTMLInputElement;
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(input.value).toBe('');
    expect(onEscape).not.toHaveBeenCalled();
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(onEscape).toHaveBeenCalledTimes(1);
  });

  it('leaves Escape alone when it has nothing to do', () => {
    render(<SearchInput label="Search" value="" onValueChange={() => {}} />);
    const event = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    screen.getByRole('searchbox').dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  });

  it('has a clear button while there is text', () => {
    render(<EmptyField />);
    expect(screen.queryByRole('button', { name: 'Clear search' })).toBeNull();
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'x' } });
    fireEvent.click(screen.getByRole('button', { name: 'Clear search' }));
    expect((screen.getByRole('searchbox') as HTMLInputElement).value).toBe('');
  });
});

describe('Toast', () => {
  it('dismisses itself after its lifetime, paused while hovered', () => {
    vi.useFakeTimers();
    const onDismiss = vi.fn();
    render(
      <LiveRegion>
        <Toast onDismiss={onDismiss} durationMs={1000}>
          Archived 14 tabs
        </Toast>
      </LiveRegion>,
    );
    expect(screen.getByRole('status').getAttribute('aria-live')).toBe('polite');
    const toast = screen.getByText('Archived 14 tabs').parentElement as HTMLElement;
    act(() => vi.advanceTimersByTime(600));
    fireEvent.mouseEnter(toast);
    act(() => vi.advanceTimersByTime(5000));
    expect(onDismiss).not.toHaveBeenCalled();
    fireEvent.mouseLeave(toast);
    act(() => vi.advanceTimersByTime(350));
    expect(onDismiss).not.toHaveBeenCalled();
    act(() => vi.advanceTimersByTime(100));
    expect(onDismiss).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });

  it('stays paused while focus moves between its own buttons', () => {
    vi.useFakeTimers();
    const onDismiss = vi.fn();
    render(
      <Toast onDismiss={onDismiss} durationMs={1000} action={<button type="button">Undo</button>}>
        Done
      </Toast>,
    );
    const undo = screen.getByRole('button', { name: 'Undo' });
    const close = screen.getByRole('button', { name: 'Dismiss' });
    fireEvent.focus(undo);
    fireEvent.blur(undo, { relatedTarget: close });
    fireEvent.focus(close);
    act(() => vi.advanceTimersByTime(5000));
    expect(onDismiss).not.toHaveBeenCalled();
    fireEvent.blur(close, { relatedTarget: document.body });
    act(() => vi.advanceTimersByTime(1000));
    expect(onDismiss).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });

  it('a new toast (new key) gets the full lifetime again', () => {
    vi.useFakeTimers();
    const onDismiss = vi.fn();
    const { rerender } = render(
      <Toast key={1} onDismiss={onDismiss} durationMs={1000}>
        First
      </Toast>,
    );
    act(() => vi.advanceTimersByTime(900));
    rerender(
      <Toast key={2} onDismiss={onDismiss} durationMs={1000}>
        Second
      </Toast>,
    );
    act(() => vi.advanceTimersByTime(900));
    expect(onDismiss).not.toHaveBeenCalled();
    act(() => vi.advanceTimersByTime(100));
    expect(onDismiss).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });

  it('can be dismissed', () => {
    const onDismiss = vi.fn();
    render(<Toast onDismiss={onDismiss}>Done</Toast>);
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    expect(onDismiss).toHaveBeenCalled();
  });
});

describe('status and text', () => {
  it('danger banners are alerts; others are status', () => {
    render(
      <>
        <Banner tone="danger" action={<Button>Fix</Button>}>
          Permission was turned off
        </Banner>
        <Banner tone="warning">Shadow mode</Banner>
      </>,
    );
    expect(screen.getByRole('alert').textContent).toContain('Permission was turned off');
    expect(screen.getByRole('status').textContent).toContain('Shadow mode');
  });

  it('dots carry their meaning in words', () => {
    render(
      <>
        <StatusDot status="down" />
        <AgeDot stage={2} />
        <Chip tone="accent">Unused 61 active h</Chip>
        <EmptyState title="Nothing archived yet">Decay is watching quietly.</EmptyState>
      </>,
    );
    expect(screen.getByText('Not connected')).toBeTruthy();
    expect(screen.getByText('Hidden')).toBeTruthy();
    expect(screen.getByText('Unused 61 active h')).toBeTruthy();
    expect(screen.getByText('Nothing archived yet')).toBeTruthy();
  });

  it('shortcuts are read as words', () => {
    render(<Shortcut accelerator="Alt+Super+D" platform="mac" />);
    expect(screen.getByRole('img', { name: 'Option Command D' }).textContent).toBe('⌥⌘D');
  });
});
