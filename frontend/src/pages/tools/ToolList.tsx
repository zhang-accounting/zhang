import { useAtomValue, useSetAtom } from 'jotai';
import { ArrowRight, FilePenLine, SearchCode, SquareStack } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';
import { PageHeader, PageShell } from '@/components/layout';
import { useDocumentTitle } from '@/hooks/use-document-title';
import { QUERY_LINK, RAW_EDITING_LINK, TOOLS_LINK } from '@/layout/nav-links';
import { cn } from '@/lib/utils';
import { breadcrumbAtom, titleAtom } from '@/states/basic';

interface ToolItem {
  /** i18n keys */
  title: string;
  description: string;
  icon: LucideIcon;
  uri: string;
}

const toolItems: ToolItem[] = [
  { title: 'tools.batch_balance_title', description: 'tools.batch_balance_description', icon: SquareStack, uri: '/tools/batch-balance' },
  { title: 'NAV_RAW_EDITING', description: 'tools.raw_edit_description', icon: FilePenLine, uri: RAW_EDITING_LINK.uri },
  { title: 'NAV_QUERY', description: 'tools.query_description', icon: SearchCode, uri: QUERY_LINK.uri },
];

export default function ToolList() {
  const { t } = useTranslation();
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`${t('NAV_TOOLS')} - ${ledgerTitle}`);
  useEffect(() => {
    setBreadcrumb([TOOLS_LINK]);
  }, [setBreadcrumb]);

  return (
    <PageShell>
      <PageHeader title={t('NAV_TOOLS')} description={t('tools.description')} />
      <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {toolItems.map((item) => (
          <li key={item.uri}>
            <Link
              to={item.uri}
              className={cn(
                'group flex h-full items-start gap-4 rounded-xl border bg-card p-4 transition-colors outline-none',
                'hover:bg-muted/40 focus-visible:ring-3 focus-visible:ring-ring/50 active:bg-muted',
              )}
            >
              <span className={cn('flex size-10 shrink-0 items-center justify-center rounded-lg', 'bg-primary/10 text-link dark:bg-primary/20')}>
                <item.icon className="size-5" />
              </span>
              <span className="flex min-w-0 flex-1 flex-col gap-1">
                <span className="font-medium">{t(item.title)}</span>
                <span className="text-sm text-muted-foreground">{t(item.description)}</span>
              </span>
              <ArrowRight className="mt-0.5 size-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
            </Link>
          </li>
        ))}
      </ul>
    </PageShell>
  );
}
