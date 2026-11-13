// Decay design system (P1.8). Styles: import "@decay/ui/theme.css" after Tailwind.
export { Banner, type BannerTone } from './Banner';
export {
  Button,
  type ButtonProps,
  type ButtonSize,
  type ButtonVariant,
  IconButton,
  type IconButtonProps,
  Spinner,
} from './Button';
export { Chip, type ChipTone, ProBadge } from './Chip';
export { cx } from './cx';
export { detectPlatform, formatShortcut, Kbd, type KeyPlatform, Shortcut } from './Kbd';
export { SearchInput } from './SearchInput';
export { type Segment, SegmentedControl } from './SegmentedControl';
export {
  AgeDot,
  type AgeStage,
  Card,
  EmptyState,
  FeatureCard,
  type Health,
  StatusDot,
} from './Status';
export { Switch } from './Switch';
export { type Tab, Tabs, tabId } from './Tabs';
export { LiveRegion, TOAST_MS, Toast } from './Toast';
export { applyTheme, effectiveTheme, isThemePreference, type ThemePreference } from './theme';
