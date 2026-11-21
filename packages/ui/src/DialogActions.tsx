import type { ReactNode } from 'react';
import { cx } from './cx';
import { detectPlatform, type KeyPlatform } from './Kbd';

export interface DialogActionsProps {
  /** The action the dialog is for (save, delete, confirm). */
  action: ReactNode;
  /** The way out (Cancel). */
  cancel: ReactNode;
  /** Classes for the row (layout: gap, alignment, wrapping). */
  className?: string;
  platform?: KeyPlatform;
}

/**
 * A dialog's action and Cancel in the platform's order (UI Design Guidelines §8): Cancel
 * then the action on macOS (the action on the right), the action then Cancel on Windows.
 */
export function DialogActions({
  action,
  cancel,
  className,
  platform = detectPlatform(),
}: DialogActionsProps) {
  return (
    <span className={cx('flex gap-2', className)}>
      {platform === 'mac' ? (
        <>
          {cancel}
          {action}
        </>
      ) : (
        <>
          {action}
          {cancel}
        </>
      )}
    </span>
  );
}
