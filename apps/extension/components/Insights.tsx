import { type InsightsProps as SharedProps, InsightsView as SharedView } from '@decay/ui/metrics';
import { PageHeader } from './PageHeader';

// Insights (Designs/14 adapted): the view is shared with the desktop app (P2.7); the
// extension's page adds its header.
export type InsightsProps = Omit<SharedProps, 'header' | 'copy' | 'embedded'>;

export function InsightsView(props: InsightsProps) {
  return <SharedView {...props} header={<PageHeader current="insights" mode={props.mode} />} />;
}
