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
});
