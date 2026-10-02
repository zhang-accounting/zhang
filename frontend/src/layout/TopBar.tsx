import { useAtomValue } from 'jotai';
import { ChevronLeft } from 'lucide-react';
import { Fragment } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';
import NewTransactionButton from '@/components/NewTransactionButton';
import { Breadcrumb, BreadcrumbItem, BreadcrumbLink, BreadcrumbList } from '@/components/ui/breadcrumb';
import { BreadcrumbPage, BreadcrumbSeparator } from '@/components/ui/breadcrumb';
import { buttonVariants } from '@/components/ui/button';
import { Separator } from '@/components/ui/separator';
import { SidebarTrigger } from '@/components/ui/sidebar';
import { cn } from '@/lib/utils';
import { breadcrumbAtom, titleAtom } from '@/states/basic';
import { LanguageSwitch } from './LanguageSwitch';
import { OnlineStatus } from './OnlineStatus';
import { ThemeToggle } from './ThemeToggle';

const HEADER_CLASS = cn(
  'sticky top-0 z-30 flex h-14 shrink-0 items-center gap-2 border-b px-3 md:px-4',
  'bg-background/90 backdrop-blur supports-backdrop-filter:bg-background/75',
);

const BACK_BUTTON_CLASS = cn(buttonVariants({ variant: 'ghost', size: 'icon' }), '-ml-1 size-10');

/**
 * Sticky top bar. >= md: sidebar toggle + breadcrumb (from `breadcrumbAtom`) + status, language, theme, new transaction.
 * < md: back button (when the breadcrumb has a parent) + current page title + status + new transaction.
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
      <SidebarTrigger className="-ml-1 hidden md:inline-flex" aria-label={t('SHELL_TOGGLE_SIDEBAR')} />
      <Separator orientation="vertical" className="mr-1 hidden data-vertical:h-4 data-vertical:self-center md:block" />

      <div className="flex min-w-0 flex-1 items-center gap-1 md:hidden">
        {parent && (
          <Link to={parent.uri} aria-label={t('SHELL_BACK')} className={BACK_BUTTON_CLASS}>
            <ChevronLeft className="size-5" />
          </Link>
        )}
        <span className="truncate text-base font-semibold">{current ? label(current) : ledgerTitle}</span>
      </div>

      <Breadcrumb className="hidden min-w-0 flex-1 md:flex">
        <BreadcrumbList className="flex-nowrap">
          {breadcrumb.map((item, index) => (
            <Fragment key={item.uri}>
              {index > 0 && <BreadcrumbSeparator />}
              <BreadcrumbItem className="min-w-0">
                {index === breadcrumb.length - 1 ? (
                  <BreadcrumbPage className="truncate">{label(item)}</BreadcrumbPage>
                ) : (
                  <BreadcrumbLink className="truncate" render={<Link to={item.uri} />}>
                    {label(item)}
                  </BreadcrumbLink>
                )}
              </BreadcrumbItem>
            </Fragment>
          ))}
        </BreadcrumbList>
      </Breadcrumb>

      <div className="ml-auto flex shrink-0 items-center gap-1">
        <OnlineStatus className="hidden md:inline-flex" showLabel />
        <OnlineStatus className="md:hidden" />
        <div className="hidden items-center md:flex">
          <LanguageSwitch />
          <ThemeToggle />
        </div>
        <NewTransactionButton />
      </div>
    </header>
  );
}
