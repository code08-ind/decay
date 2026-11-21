// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { applyChrome, chromeFlags, TitleBar } from './TitleBar';

let resized: () => void = () => {};
const win = {
  title: vi.fn(async () => 'Settings'),
  isMaximized: vi.fn(async () => false),
  onResized: vi.fn(async (f: () => void) => {
    resized = f;
    return () => {};
  }),
  minimize: vi.fn(async () => {}),
  toggleMaximize: vi.fn(async () => {}),
  close: vi.fn(async () => {}),
};
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => win }));

afterEach(cleanup);

describe('chrome flags', () => {
  it('reads what Rust put in the query', () => {
    expect(chromeFlags('')).toEqual({ custom: false, backdrop: null });
    expect(chromeFlags('?chrome=custom')).toEqual({ custom: true, backdrop: null });
    expect(chromeFlags('?chrome=custom&backdrop=mica')).toEqual({ custom: true, backdrop: 'mica' });
    expect(chromeFlags('?backdrop=acrylic')).toEqual({ custom: false, backdrop: 'acrylic' });
    expect(chromeFlags('?backdrop=blur')).toEqual({ custom: false, backdrop: null });
  });

  it('marks the page for the CSS', () => {
    const plain = document.createElement('html');
    applyChrome(chromeFlags(''), plain);
    expect({ ...plain.dataset }).toEqual({});
    const mica = document.createElement('html');
    applyChrome(chromeFlags('?chrome=custom&backdrop=mica'), mica);
    expect({ ...mica.dataset }).toEqual({ chrome: 'custom', backdrop: 'mica' });
  });
});

describe('TitleBar', () => {
  it("shows the window's title and drives the window", async () => {
    render(<TitleBar />);
    expect(await screen.findByText('Settings')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Minimize' }));
    expect(win.minimize).toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Maximize' }));
    expect(win.toggleMaximize).toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(win.close).toHaveBeenCalled();
    // The caption buttons stay out of the Tab order, like the system's.
    for (const b of screen.getAllByRole('button')) expect(b.tabIndex).toBe(-1);
  });

  it('turns Maximize into Restore once maximised', async () => {
    render(<TitleBar />);
    await screen.findByRole('button', { name: 'Maximize' });
    win.isMaximized.mockResolvedValueOnce(true);
    await act(async () => resized());
    expect(await screen.findByRole('button', { name: 'Restore down' })).toBeTruthy();
  });

  it('a fixed-size window only has Close', async () => {
    render(<TitleBar resizable={false} />);
    await screen.findByText('Settings');
    expect(screen.getAllByRole('button').map((b) => b.getAttribute('aria-label'))).toEqual([
      'Close',
    ]);
  });
});
