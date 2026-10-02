import { Loader2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { cn } from '@/lib/utils';

/**
 * "Updating…" with a spinner, announced politely. Use it (e.g. as the `PageHeader` description) while a page refetches for a
 * new range / month and still shows the previous numbers, together with `aria-busy` + dimming on the stale content.
 */
export function RefreshingLabel({ className }: { className?: string }) {
  const { t } = useTranslation();
  return (
    <span role="status" className={cn('inline-flex items-center gap-1.5', className)}>
      <Loader2 className="size-3.5 motion-safe:animate-spin" aria-hidden />
      {t('ledger.common.updating')}
    </span>
  );
}
