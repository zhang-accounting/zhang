import { useAtomValue } from 'jotai';
import { TriangleAlert } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { reloadFailureAtom } from '@/states/basic';
import { reloadFailureDetail, reloadFailureEditorHref } from './reload-failure';

const NOTICE_CLASS = cn('flex flex-col gap-3 rounded-lg border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm sm:flex-row sm:items-start');

/**
 * Banner shown on every page while the last reload failed (`reload_failure` of `/api/info`, refreshed on the SSE `ReloadFailed`
 * and `Reload` events): the server keeps serving the ledger loaded before, so the pages show that version until the file is
 * fixed. The link opens the raw editor on the failing file. Goes away with the next reload that succeeds (#492).
 */
export function ReloadFailureNotice() {
  const { t } = useTranslation();
  const failure = useAtomValue(reloadFailureAtom);
  if (!failure) return null;

  return (
    <div className="mx-auto w-full max-w-7xl px-4 pt-4 md:px-7 md:pt-5">
      <div role="alert" className={NOTICE_CLASS}>
        <TriangleAlert className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden />
        <div className="min-w-0 flex-1 space-y-1">
          <p className="font-medium">{t('SHELL_RELOAD_FAILED')}</p>
          <p className="break-words font-mono text-xs text-foreground-2">{reloadFailureDetail(failure)}</p>
          <p className="text-foreground-2">{t('SHELL_RELOAD_FAILED_STALE')}</p>
        </div>
        <Link to={reloadFailureEditorHref(failure)} className={cn(buttonVariants({ variant: 'outline', size: 'sm' }), 'shrink-0')}>
          {t('SHELL_RELOAD_FAILED_OPEN_EDITOR')}
        </Link>
      </div>
    </div>
  );
}
