import { useAtomValue } from 'jotai';
import { ChevronLeft } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';
import NewTransactionButton from '@/components/NewTransactionButton';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { breadcrumbAtom, titleAtom } from '@/states/basic';

const HEADER_CLASS = cn(
  'sticky top-0 z-30 flex h-14 shrink-0 items-center gap-2 border-b px-4 md:hidden',
  'bg-background/90 backdrop-blur supports-backdrop-filter:bg-background/75',
);

const BACK_BUTTON_CLASS = cn(buttonVariants({ variant: 'ghost', size: 'icon' }), '-ml-2 size-10');

/**
 * Mobile (< md) top bar: the otter + ledger title on top-level pages (their `<h1>` follows right below), a back link + the page
 * title on nested pages (from `breadcrumbAtom`), and the turquoise "+" new-transaction button. Desktop has no top bar: the
 * sidebar holds the shell controls and `PageHeader` shows the breadcrumb trail.
 */
export function TopBar() {
  const { t } = useTranslation();
  const breadcrumb = useAtomValue(breadcrumbAtom);
  const ledgerTitle = useAtomValue(titleAtom);
  const label = (item: (typeof breadcrumb)[number]) => ((item.noTranslate ?? false) ? item.label : t(item.label));
  const current = breadcrumb[breadcrumb.length - 1];
  const parent = breadcrumb.length > 1 ? breadcrumb[breadcrumb.length - 2] : undefined;

  return (
    <header className={HEADER_CLASS}>
      <div className="flex min-w-0 flex-1 items-center gap-2">
        {parent ? (
          <Link to={parent.uri} aria-label={t('SHELL_BACK')} className={BACK_BUTTON_CLASS}>
            <ChevronLeft className="size-5" />
          </Link>
        ) : (
          <img src="/otter-192.png" alt="" className="size-6 shrink-0 rounded-md" />
        )}
        <span className="truncate text-base font-semibold">{parent && current ? label(current) : ledgerTitle}</span>
      </div>
      <NewTransactionButton variant="icon" />
    </header>
  );
}
