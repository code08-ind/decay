// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AppLink } from './AppLink';

describe('AppLink', () => {
  afterEach(cleanup);

  it('offers to connect when the link is off', () => {
    const onConnect = vi.fn();
    render(<AppLink status={null} enabled={false} onConnect={onConnect} onDisconnect={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    expect(onConnect).toHaveBeenCalled();
  });

  it('shows the link state and offers to disconnect', () => {
    const onDisconnect = vi.fn();
    const { rerender } = render(
      <AppLink
        status={{ state: 'connected', at: 1 }}
        enabled
        onConnect={vi.fn()}
        onDisconnect={onDisconnect}
      />,
    );
    expect(screen.getByText(/Connected to the Decay app/)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Disconnect' }));
    expect(onDisconnect).toHaveBeenCalled();
    rerender(
      <AppLink
        status={{ state: 'unavailable', at: 1 }}
        enabled
        onConnect={vi.fn()}
        onDisconnect={vi.fn()}
      />,
    );
    expect(screen.getByText(/isn’t running/)).toBeTruthy();
    rerender(
      <AppLink
        status={{ state: 'outdated', at: 1 }}
        enabled
        onConnect={vi.fn()}
        onDisconnect={vi.fn()}
      />,
    );
    expect(screen.getByText(/Update this extension/)).toBeTruthy();
  });

  it('names this browser profile while connected (Enter saves, Escape keeps it)', () => {
    const onRename = vi.fn();
    const props = {
      status: { state: 'connected' as const, at: 1 },
      enabled: true,
      onConnect: vi.fn(),
      onDisconnect: vi.fn(),
      onRename,
    };
    const { rerender } = render(<AppLink {...props} profile={{ ordinal: 2, hasSiblings: true }} />);
    expect(screen.getByText('This profile: Profile 2')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Rename' }));
    const input = screen.getByRole('textbox', { name: 'Name for this browser profile' });
    fireEvent.change(input, { target: { value: ' Work ' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onRename).toHaveBeenLastCalledWith('Work');
    rerender(<AppLink {...props} profile={{ name: 'Work', ordinal: 2, hasSiblings: true }} />);
    expect(screen.getByText('This profile: Work')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Rename' }));
    const again = screen.getByRole('textbox', { name: 'Name for this browser profile' });
    fireEvent.change(again, { target: { value: 'Oops' } });
    fireEvent.keyDown(again, { key: 'Escape' });
    expect(onRename).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole('button', { name: 'Rename' }));
    const cleared = screen.getByRole('textbox', { name: 'Name for this browser profile' });
    fireEvent.change(cleared, { target: { value: '' } });
    fireEvent.blur(cleared);
    expect(onRename).toHaveBeenLastCalledWith(null);
    // Not connected: no profile line.
    rerender(
      <AppLink
        {...props}
        status={null}
        enabled={false}
        profile={{ ordinal: 1, hasSiblings: false }}
      />,
    );
    expect(screen.queryByText(/This profile/)).toBeNull();
  });
});
