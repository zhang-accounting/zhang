import { useAtomValue } from 'jotai';
import { ChevronRight } from 'lucide-react';
import * as React from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router';
import { cn } from '@/lib/utils';
import { breadcrumbAtom } from '@/states/basic';

export interface PageHeaderProps {
  title: React.ReactNode;
  description?: React.ReactNode;
  /** Buttons / menus aligned right on >= sm, wrapped under the title on mobile. */
  actions?: React.ReactNode;
  /** Optional second row (filters, tabs, summary chips). */
  children?: React.ReactNode;
  className?: string;
}

/**
 * Small trail above the `<h1>` on nested pages (>= md only: there is no desktop top bar, and the mobile top bar shows a back
 * link instead). Comes from `breadcrumbAtom`, which `PageMeta` (router.tsx) sets from the route; the last crumb (the page itself) is plain text.
 */
function BreadcrumbTrail() {
  const { t } = useTranslation();
  const breadcrumb = useAtomValue(breadcrumbAtom);
  if (breadcrumb.length < 2) return null;
  const label = (item: (typeof breadcrumb)[number]) => ((item.noTranslate ?? false) ? item.label : t(item.label));
  return (
    <nav aria-label={t('SHELL_BREADCRUMB')} className="hidden min-w-0 md:block">
      <ol className="flex min-w-0 items-center gap-1 text-xs text-muted-foreground">
        {breadcrumb.slice(0, -1).map((item) => (
          <li key={item.uri} className="flex min-w-0 items-center gap-1">
            <Link to={item.uri} className="truncate rounded-sm hover:text-foreground hover:underline hover:underline-offset-4">
              {label(item)}
            </Link>
            <ChevronRight className="size-3 shrink-0" aria-hidden />
          </li>
        ))}
        <li className="min-w-0 truncate text-foreground-2" aria-current="page">
          {label(breadcrumb[breadcrumb.length - 1])}
        </li>
      </ol>
    </nav>
  );
}

/** Page title block. Renders an `<h1>`; keep one per page. */
export function PageHeader({ title, description, actions, children, className }: PageHeaderProps) {
  return (
    <header data-slot="page-header" className={cn('flex flex-col gap-3', className)}>
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="flex min-w-0 flex-col gap-1">
          <BreadcrumbTrail />
          <h1 className="truncate text-xl font-semibold tracking-tight">{title}</h1>
          {description && <p className="text-sm text-muted-foreground">{description}</p>}
        </div>
        {actions && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
      </div>
      {children}
    </header>
  );
}
