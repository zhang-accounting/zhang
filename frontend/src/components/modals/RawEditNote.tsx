import { TriangleAlert } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router';
import { RAW_EDIT_URI } from '@/lib/raw-edit-link';
import { cn } from '@/lib/utils';

/** Inline warning for transactions the form cannot rewrite without losing data (cost / price), pointing to Raw Edit. */
export function RawEditNote({ id, className }: { id?: string; className?: string }) {
  const { t } = useTranslation();
  return (
    <div id={id} role="note" className={cn('flex gap-2 rounded-lg border border-warning/30 bg-warning/10 p-3 text-sm text-warning', className)}>
      <TriangleAlert className="mt-0.5 size-4 shrink-0" aria-hidden />
      <p>
        {t('ledger.txn.edit_blocked')}{' '}
        <Link to={RAW_EDIT_URI} className="font-medium underline underline-offset-4">
          {t('ledger.txn.open_raw_edit')}
        </Link>
      </p>
    </div>
  );
}
