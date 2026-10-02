import { ChevronLeft, ChevronRight } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

interface Props {
  page: number;
  totalPages: number;
  onPageChange: (page: number) => void;
  className?: string;
}

/** Window of page numbers around `page`, always including the first and last page (`0` marks a gap). */
function pageWindow(page: number, totalPages: number): number[] {
  const pages = new Set([1, totalPages, page - 1, page, page + 1].filter((it) => it >= 1 && it <= totalPages));
  const sorted = [...pages].sort((a, b) => a - b);
  return sorted.flatMap((it, index) => (index > 0 && it - sorted[index - 1] > 1 ? [0, it] : [it]));
}

/** Prev / next with a page indicator on mobile; numbered pages on >= sm. Buttons are 40px tall on touch screens. */
export function PagePagination({ page, totalPages, onPageChange, className }: Props) {
  const { t } = useTranslation();
  if (totalPages <= 1) return null;

  return (
    <nav aria-label={t('ledger.pagination.label')} className={cn('flex items-center justify-between gap-2', className)}>
      <p className="hidden text-sm text-muted-foreground tabular-nums sm:block">{t('ledger.pagination.page_of', { page, total: totalPages })}</p>
      <div className="flex w-full items-center justify-between gap-1 sm:w-auto sm:justify-end">
        <Button variant="outline" className="h-10 md:h-8" disabled={page <= 1} onClick={() => onPageChange(page - 1)}>
          <ChevronLeft data-icon="inline-start" />
          {t('PAGINATION_PREVIOUS')}
        </Button>
        <span className="text-sm text-muted-foreground tabular-nums sm:hidden">
          {page} / {totalPages}
        </span>
        <div className="hidden items-center gap-1 sm:flex">
          {pageWindow(page, totalPages).map((it, index) =>
            it === 0 ? (
              <span key={`gap-${index}`} className="w-6 text-center text-muted-foreground">
                …
              </span>
            ) : (
              <Button
                key={it}
                variant={it === page ? 'outline' : 'ghost'}
                size="icon"
                className="w-auto min-w-8 px-2 tabular-nums"
                aria-current={it === page ? 'page' : undefined}
                onClick={() => onPageChange(it)}
              >
                {it}
              </Button>
            ),
          )}
        </div>
        <Button variant="outline" className="h-10 md:h-8" disabled={page >= totalPages} onClick={() => onPageChange(page + 1)}>
          {t('PAGINATION_NEXT')}
          <ChevronRight data-icon="inline-end" />
        </Button>
      </div>
    </nav>
  );
}
