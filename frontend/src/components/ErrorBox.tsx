import { useAtomValue, useSetAtom } from 'jotai';
import { ChevronLeft, ChevronRight, FilePenLine, FileWarning, TriangleAlert } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router';
import { LedgerError } from '@/api/types';
import { useDisclosure } from '@/hooks/use-disclosure';
import { rawEditLink } from '@/lib/raw-edit-link';
import { cn } from '@/lib/utils';
import { errorAtom, errorPageAtom } from '../states/errors';
import { errorLocation } from './error-location';
import { Badge } from './ui/badge';
import { Button, buttonVariants } from './ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from './ui/dialog';
import { Item, ItemContent, ItemDescription, ItemGroup, ItemMedia, ItemTitle } from './ui/item';
import { Skeleton } from './ui/skeleton';

/**
 * Ledger error list (Home): tappable rows opening a detail dialog, compact pager, the otter + "healthy" when empty.
 *
 * The dialog shows the directive as it is in the file and opens the file in Raw Editing at that line (#493): the fix is
 * often elsewhere in the file (an account that was never opened, a commodity that is not declared), so the directive is
 * not edited in place.
 */
export default function ErrorBox() {
  const { t } = useTranslation();
  const [isOpen, isOpenHandler] = useDisclosure(false);

  const [selectError, setSelectError] = useState<LedgerError | null>(null);

  const errors = useAtomValue(errorAtom);
  const setErrorPage = useSetAtom(errorPageAtom);

  if (errors.state === 'loading' || errors.state === 'hasError') {
    return (
      <div className="flex flex-col gap-2" aria-busy>
        {Array.from({ length: 4 }, (_, index) => (
          <div key={index} className="flex items-center gap-3 px-3 py-2">
            <Skeleton className="size-8 rounded-lg" />
            <div className="flex flex-1 flex-col gap-1.5">
              <Skeleton className="h-4 w-3/4" />
              <Skeleton className="h-3 w-1/3" />
            </div>
          </div>
        ))}
      </div>
    );
  }

  // a title can show the error's metas, e.g. `{{meta.message}}` for the problem a plugin describes
  const errorTitle = (error: LedgerError) => t(`ERROR.${error.error_type}`, { defaultValue: error.error_type, meta: error.metas });

  const toggleError = (error: LedgerError) => {
    setSelectError(error);
    isOpenHandler.open();
  };

  const { current_page: currentPage, total_page: totalPage, total_count: totalCount, records } = errors.data;
  const metas = Object.entries(selectError?.metas ?? {});
  const span = selectError?.span;
  // nothing to open for an error without a file (a directive a plugin generated); without a line the file opens at its start
  const editorLink = rawEditLink(span?.filename, span?.line);

  return (
    <>
      <Dialog open={isOpen} onOpenChange={() => isOpenHandler.close()}>
        <DialogContent className="max-h-[90svh] overflow-y-auto sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2 pr-6">
              <TriangleAlert className="size-4 shrink-0 text-destructive" />
              {selectError && errorTitle(selectError)}
            </DialogTitle>
            <DialogDescription className="font-mono text-xs break-all">{errorLocation(span, t)}</DialogDescription>
          </DialogHeader>
          {metas.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {metas.map(([key, value]) => (
                <Badge key={key} variant="outline" className="max-w-full font-mono">
                  <span className="text-muted-foreground">{key}</span>
                  <span className="truncate">{value}</span>
                </Badge>
              ))}
            </div>
          )}
          {span && span.content !== '' && (
            <pre className="max-h-[50svh] overflow-auto rounded-lg bg-muted p-3 font-mono text-xs leading-relaxed break-all whitespace-pre-wrap">
              {span.content}
            </pre>
          )}
          {editorLink && (
            <DialogFooter>
              <Link to={editorLink} className={cn(buttonVariants(), 'h-10 md:h-8')}>
                <FilePenLine />
                {t('ERROR_BOX_OPEN_EDITOR')}
              </Link>
            </DialogFooter>
          )}
        </DialogContent>
      </Dialog>

      {totalCount === 0 ? (
        <div className="flex flex-col items-center gap-3 py-4 text-center">
          <img className="size-30 rounded-lg" src="/otter-512.png" alt="" />
          <p className="text-base font-semibold">{t('LEDGER_IS_HEALTHY')}</p>
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          <ItemGroup className="gap-1">
            {/* errors on one span share an id (a plugin reports all of its errors without a span on its directive) */}
            {records.map((error, index) => (
              <Item
                key={`${error.id}-${index}`}
                size="sm"
                render={<button type="button" />}
                className="min-h-12 cursor-pointer flex-nowrap text-left hover:bg-muted/60 active:bg-muted"
                onClick={() => toggleError(error)}
              >
                <ItemMedia variant="icon" className="size-8 rounded-lg bg-destructive/10 text-destructive">
                  <FileWarning />
                </ItemMedia>
                <ItemContent className="min-w-0">
                  <ItemTitle className="line-clamp-2 w-full">{errorTitle(error)}</ItemTitle>
                  {error.span && <ItemDescription className="truncate font-mono text-xs">{errorLocation(error.span, t)}</ItemDescription>}
                </ItemContent>
                <ChevronRight className="size-4 shrink-0 text-muted-foreground" />
              </Item>
            ))}
          </ItemGroup>

          {totalPage > 1 && (
            <nav aria-label={t('PAGE')} className="flex items-center justify-between gap-2 border-t pt-3">
              <Button variant="ghost" className="h-10 md:h-8" disabled={currentPage <= 1} onClick={() => setErrorPage(currentPage - 1)}>
                <ChevronLeft />
                {t('PAGINATION_PREVIOUS')}
              </Button>
              <span className="text-xs text-muted-foreground tabular-nums">
                {currentPage} / {totalPage}
              </span>
              <Button variant="ghost" className="h-10 md:h-8" disabled={currentPage >= totalPage} onClick={() => setErrorPage(currentPage + 1)}>
                {t('PAGINATION_NEXT')}
                <ChevronRight />
              </Button>
            </nav>
          )}
        </div>
      )}
    </>
  );
}
