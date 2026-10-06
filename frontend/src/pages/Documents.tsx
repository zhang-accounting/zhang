import { format } from 'date-fns';
import { groupBy, sortBy } from 'lodash-es';
import { ExternalLink, FileStack, FileText, ImageIcon, LayoutGrid, List } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router';
import { useAsyncRetry } from 'react-use';
import { retrieveDocuments } from '@/api/requests';
import { Document } from '@/api/types';
import AccountDocumentLine from '@/components/documentLines/AccountDocumentLine';
import { documentUrl } from '@/components/documentLines/document-utils';
import { DocumentUploadDialog } from '@/components/documentLines/DocumentUploadDialog';
import { ImageLightBox } from '@/components/ImageLightBox';
import { OpenInExplore } from '@/components/query/OpenInExplore';
import { EmptyState, LoadFailedState, PageHeader, PageShell, ResponsiveList, type ResponsiveColumn } from '@/components/layout';
import { useDateFormat } from '@/components/layout/use-date-format';
import { Badge } from '@/components/ui/badge';
import { Button, buttonVariants } from '@/components/ui/button';
import { Skeleton } from '@/components/ui/skeleton';
import { useLocalStorage } from '@/hooks/use-local-storage';
import { cn } from '@/lib/utils';
import { canPreview, documentType } from '@/utils/documents';

const GRID_CLASS = 'grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6';

function LayoutToggle({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  const { t } = useTranslation();
  const options = [
    { value: 'Grid', label: t('documents.layout_grid'), icon: LayoutGrid },
    { value: 'Table', label: t('documents.layout_list'), icon: List },
  ];
  return (
    <div role="group" aria-label={t('documents.layout')} className="flex rounded-lg bg-muted p-0.5">
      {options.map((option) => (
        <Button
          key={option.value}
          variant="ghost"
          aria-pressed={value === option.value}
          aria-label={option.label}
          title={option.label}
          className={cn('h-10 px-2.5 md:h-7', value === option.value && 'bg-background shadow-xs hover:bg-background dark:bg-input/40')}
          onClick={() => onChange(option.value)}
        >
          <option.icon />
          <span className="hidden sm:inline">{option.label}</span>
        </Button>
      ))}
    </div>
  );
}

function LinkedTo({ document }: { document: Document }) {
  return (
    <div className="flex min-w-0 flex-wrap gap-1" onClick={(event) => event.stopPropagation()}>
      {document.account && (
        <Badge variant="outline" className="max-w-full" render={<Link to={`/accounts/${document.account}`} />}>
          <span className="truncate">{document.account}</span>
        </Badge>
      )}
      {document.trx_id && (
        <Badge variant="secondary" className="font-mono" title={document.trx_id}>
          {document.trx_id.slice(0, 8)}
        </Badge>
      )}
    </div>
  );
}

export default function Documents() {
  const { t } = useTranslation();
  const fmt = useDateFormat();
  const [layout, setLayout] = useLocalStorage({ key: `document-list-layout`, defaultValue: 'Grid' });
  const [lightboxSrc, setLightboxSrc] = useState<string | undefined>(undefined);

  const {
    loading,
    error,
    value: documents,
    retry,
  } = useAsyncRetry(async () => {
    const res = await retrieveDocuments({});
    return res.data.data;
  }, []);

  const sortedDocuments = useMemo(() => sortBy(documents ?? [], (document) => -new Date(document.datetime).getTime()), [documents]);
  const groupedDocuments = useMemo(
    () => Object.entries(groupBy(sortedDocuments, (document) => format(new Date(document.datetime), 'yyyy-MM'))),
    [sortedDocuments],
  );

  const open = (document: Document) => {
    if (canPreview(document)) setLightboxSrc(document.path);
    else window.open(documentUrl(document.path), '_blank', 'noopener');
  };

  const columns: ResponsiveColumn<Document>[] = [
    {
      key: 'file',
      header: t('documents.file'),
      cell: (document) => (
        <div className="flex min-w-0 items-center gap-2">
          {canPreview(document) ? (
            <ImageIcon className="size-4 shrink-0 text-muted-foreground" />
          ) : (
            <FileText className="size-4 shrink-0 text-muted-foreground" />
          )}
          <span className="truncate font-medium">{document.filename}</span>
        </div>
      ),
    },
    { key: 'linked', header: t('documents.linked_to'), cell: (document) => <LinkedTo document={document} /> },
    { key: 'type', header: t('documents.type'), className: 'w-20 text-muted-foreground', cell: (document) => documentType(document) || '—' },
    {
      key: 'date',
      header: t('documents.date'),
      className: 'w-44 text-muted-foreground tabular-nums',
      cell: (document) => format(new Date(document.datetime), 'yyyy-MM-dd HH:mm'),
    },
    {
      key: 'open',
      header: <span className="sr-only">{t('documents.open')}</span>,
      className: 'w-12 text-right',
      cell: (document) => (
        <a
          href={documentUrl(document.path)}
          target="_blank"
          rel="noreferrer"
          onClick={(event) => event.stopPropagation()}
          aria-label={t('documents.open')}
          className={cn(buttonVariants({ variant: 'ghost', size: 'icon' }))}
        >
          <ExternalLink />
        </a>
      ),
    },
  ];

  const firstLoad = loading && documents === undefined;
  const empty = <EmptyState icon={FileStack} title={t('documents.empty_title')} description={t('documents.empty_description')} />;

  return (
    <PageShell>
      <PageHeader
        title={t('NAV_DOCUMENTS')}
        description={documents && documents.length > 0 ? t('documents.description_count', { count: documents.length }) : t('documents.description')}
        actions={
          <>
            <LayoutToggle value={layout} onChange={setLayout} />
            <OpenInExplore name="journals.documents" />
            <DocumentUploadDialog onUploaded={retry} />
          </>
        }
      />
      <ImageLightBox src={lightboxSrc} onChange={setLightboxSrc} />

      {error ? (
        <LoadFailedState description={error.message} onRetry={retry} />
      ) : firstLoad && layout === 'Grid' ? (
        <div className={GRID_CLASS}>
          {Array.from({ length: 8 }, (_, index) => (
            <div key={index} className="flex flex-col gap-2">
              <Skeleton className="aspect-[4/3] w-full rounded-xl" />
              <Skeleton className="h-4 w-3/4" />
            </div>
          ))}
        </div>
      ) : sortedDocuments.length === 0 && !firstLoad ? (
        empty
      ) : layout === 'Grid' ? (
        <div className="flex flex-col gap-6">
          {groupedDocuments.map(([month, monthDocuments]) => (
            <section key={month} className="flex flex-col gap-3">
              <h2 className="flex items-baseline gap-2 text-sm font-semibold">
                {fmt.month(new Date(monthDocuments[0].datetime))}
                <span className="text-xs font-normal text-muted-foreground tabular-nums">{monthDocuments.length}</span>
              </h2>
              <div className={GRID_CLASS}>
                {monthDocuments.map((document) => (
                  <AccountDocumentLine key={document.path} onClick={setLightboxSrc} {...document} />
                ))}
              </div>
            </section>
          ))}
        </div>
      ) : (
        <ResponsiveList
          items={sortedDocuments}
          loading={firstLoad}
          getKey={(document) => document.path}
          columns={columns}
          onItemClick={open}
          renderCard={(document) => (
            <div className="flex items-center gap-3">
              <span className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
                {canPreview(document) ? <ImageIcon className="size-4" /> : <FileText className="size-4" />}
              </span>
              <div className="min-w-0 flex-1">
                <div className="truncate font-medium">{document.filename}</div>
                <div className="truncate text-xs text-muted-foreground">
                  {format(new Date(document.datetime), 'yyyy-MM-dd')}
                  {document.account && ` · ${document.account}`}
                </div>
              </div>
            </div>
          )}
          empty={empty}
        />
      )}
    </PageShell>
  );
}
