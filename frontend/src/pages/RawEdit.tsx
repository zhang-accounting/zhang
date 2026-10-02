import { useAtomValue, useSetAtom } from 'jotai';
import { FileText, FolderTree, TriangleAlert } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';
import { useAsync } from 'react-use';
import { retrieveFiles } from '@/api/requests';
import { buildFileTree } from '@/components/basic/file-tree';
import { FileTree, TableOfContentsFloating } from '@/components/basic/TableOfContentsFloating';
import { EmptyState, PageHeader, PageShell } from '@/components/layout';
import SingleFileEdit from '@/components/SingleFileEdit';
import { Skeleton } from '@/components/ui/skeleton';
import { useDocumentTitle } from '@/hooks/use-document-title';
import { RAW_EDITING_LINK } from '@/layout/nav-links';
import { cn } from '@/lib/utils';
import { breadcrumbAtom, titleAtom } from '@/states/basic';

/**
 * The page fills the viewport below the top bar: 100svh - top bar (3.5rem) - shell paddings
 * (mobile: 1rem top + 5rem bottom incl. the 4rem tab bar; desktop: 1.5rem + 1.5rem), so the save bar sits just above the tab bar.
 */
const VIEWPORT_HEIGHT = 'h-[calc(100svh-3.5rem-6rem-env(safe-area-inset-bottom))] md:h-[calc(100svh-3.5rem-3rem)] min-h-[26rem]';

function RawEdit() {
  const { t } = useTranslation();
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const [searchParams, setSearchParams] = useSearchParams();
  const [dirty, setDirty] = useState(false);
  const {
    loading,
    error,
    value: files,
  } = useAsync(async () => {
    const res = await retrieveFiles({});
    return (res.data.data ?? []).filter((it): it is string => it !== null);
  }, []);

  const tree = useMemo(() => buildFileTree(files ?? []), [files]);
  const requested = searchParams.get('file');
  const selectedFile = requested && files?.includes(requested) ? requested : (files?.[0] ?? null);

  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(selectedFile ? `${selectedFile} | ${t('NAV_RAW_EDITING')} - ${ledgerTitle}` : `${t('NAV_RAW_EDITING')} - ${ledgerTitle}`);
  useEffect(() => {
    setBreadcrumb([RAW_EDITING_LINK]);
  }, [setBreadcrumb]);

  const selectFile = (file: string) => {
    if (file === selectedFile) return;
    if (dirty && !window.confirm(t('raw_edit.discard_confirm'))) return;
    setDirty(false);
    // Replace, not push: Back cannot be guarded (BrowserRouter), so it should leave the editor rather than silently swap files.
    setSearchParams({ file }, { replace: true });
  };
  const dirtyPath = dirty ? selectedFile : null;

  return (
    <PageShell className={cn(VIEWPORT_HEIGHT, 'gap-3 md:gap-4')}>
      <PageHeader title={t('NAV_RAW_EDITING')} description={t('raw_edit.description')} />

      {error ? (
        <EmptyState icon={TriangleAlert} title={t('page_state.load_failed')} description={error.message} />
      ) : !loading && (files ?? []).length === 0 ? (
        <EmptyState icon={FolderTree} title={t('raw_edit.no_files_title')} description={t('raw_edit.no_files_description')} />
      ) : (
        <div className="flex min-h-0 flex-1 gap-4">
          <aside className="hidden w-60 shrink-0 flex-col overflow-hidden rounded-xl border bg-card md:flex lg:w-64">
            <div className="flex items-center justify-between border-b px-3 py-2 text-xs font-medium text-muted-foreground">
              <span>{t('raw_edit.files')}</span>
              <span className="tabular-nums">{files?.length ?? ''}</span>
            </div>
            {loading ? (
              <div className="flex flex-col gap-2 p-3">
                <Skeleton className="h-5 w-3/4" />
                <Skeleton className="h-5 w-1/2" />
              </div>
            ) : (
              <FileTree files={tree} selected={selectedFile} dirtyPath={dirtyPath} onChange={selectFile} className="min-h-0 flex-1 overflow-y-auto p-1.5" />
            )}
          </aside>

          <section className="flex min-w-0 flex-1 flex-col overflow-hidden rounded-xl border bg-card">
            <div className="flex shrink-0 items-center gap-2 border-b p-2 md:px-3">
              <TableOfContentsFloating files={tree} selected={selectedFile} dirtyPath={dirtyPath} onChange={selectFile} className="w-full md:hidden" />
              <div className="hidden min-w-0 flex-1 items-center gap-2 text-sm md:flex">
                <FileText className="size-4 shrink-0 text-muted-foreground" />
                <span className="truncate font-mono text-xs">{selectedFile ?? t('raw_edit.choose_file')}</span>
                {dirty && <span className="size-2 shrink-0 rounded-full bg-amber-500" aria-label={t('raw_edit.unsaved')} />}
              </div>
            </div>
            {selectedFile ? (
              <SingleFileEdit key={selectedFile} name={selectedFile} path={selectedFile} onDirtyChange={setDirty} className="flex-1" />
            ) : (
              <div className="flex flex-col gap-2 p-4">
                <Skeleton className="h-4 w-1/2" />
                <Skeleton className="h-4 w-2/3" />
              </div>
            )}
          </section>
        </div>
      )}
    </PageShell>
  );
}

export default RawEdit;
