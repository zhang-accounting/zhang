import * as React from 'react';
import { Link } from 'react-router-dom';
import { Skeleton } from '@/components/ui/skeleton';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { useIsMobile } from '@/hooks/use-mobile';
import { cn } from '@/lib/utils';

export interface ResponsiveColumn<T> {
  key: string;
  header: React.ReactNode;
  cell: (item: T, index: number) => React.ReactNode;
  /** Applied to both `<th>` and `<td>`, e.g. `'text-right tabular-nums'` or `'w-32'`. */
  className?: string;
}

export interface ResponsiveListProps<T> {
  items: T[];
  getKey: (item: T, index: number) => React.Key;
  /** Desktop (>= md) table columns. */
  columns: ResponsiveColumn<T>[];
  /** Mobile (< md) row content; rows are stacked in a bordered card list. */
  renderCard: (item: T, index: number) => React.ReactNode;
  /**
   * Rows that navigate: renders real `<Link>`s so keyboard, screen readers and Cmd/middle-click work. Desktop: the `linkColumn`
   * cell (default: first column) holds the link, stretched over the whole row; mobile: the whole card is the link.
   * Takes precedence over `onItemClick` (keep that for in-page actions such as opening a preview).
   */
  getItemHref?: (item: T, index: number) => string;
  /** Column key whose cell carries the row link (default: the first column). */
  linkColumn?: string;
  onItemClick?: (item: T, index: number) => void;
  loading?: boolean;
  /** Rendered instead of the list when `items` is empty and not loading (e.g. `<EmptyState />`). */
  empty?: React.ReactNode;
  className?: string;
}

const FOCUS_RING = 'outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset';
const CARD_CLASS = cn('block min-h-12 w-full p-4 text-left transition-colors active:bg-muted', FOCUS_RING);
/** Stretched link: its `::after` covers the (relative) row, so the whole row is clickable and shows the focus ring. */
const ROW_LINK_CLASS =
  'block min-w-0 outline-none after:absolute after:inset-0 focus-visible:after:ring-2 focus-visible:after:ring-ring focus-visible:after:ring-inset';

/** One data set, two layouts: a table on desktop and tappable card rows on mobile (switches at md / 768px). */
export function ResponsiveList<T>(props: ResponsiveListProps<T>) {
  const { items, getKey, columns, renderCard, getItemHref, linkColumn, onItemClick, loading = false, empty, className } = props;
  const isMobile = useIsMobile();
  const linkKey = linkColumn ?? columns[0]?.key;

  if (!loading && items.length === 0 && empty) return <>{empty}</>;

  if (isMobile) {
    return (
      <ul data-slot="responsive-list" className={cn('divide-y overflow-hidden rounded-xl border bg-card', className)}>
        {loading
          ? Array.from({ length: 3 }, (_, index) => (
              <li key={index} className="p-4">
                <Skeleton className="h-5 w-2/3" />
                <Skeleton className="mt-2 h-4 w-1/3" />
              </li>
            ))
          : items.map((item, index) => (
              <li key={getKey(item, index)}>
                {getItemHref ? (
                  <Link to={getItemHref(item, index)} className={CARD_CLASS}>
                    {renderCard(item, index)}
                  </Link>
                ) : onItemClick ? (
                  <button type="button" className={CARD_CLASS} onClick={() => onItemClick(item, index)}>
                    {renderCard(item, index)}
                  </button>
                ) : (
                  <div className="min-h-12 p-4">{renderCard(item, index)}</div>
                )}
              </li>
            ))}
      </ul>
    );
  }

  return (
    <div data-slot="responsive-list" className={cn('overflow-hidden rounded-xl border bg-card', className)}>
      <Table>
        <TableHeader>
          <TableRow>
            {columns.map((column) => (
              <TableHead key={column.key} className={column.className}>
                {column.header}
              </TableHead>
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {loading
            ? Array.from({ length: 3 }, (_, index) => (
                <TableRow key={index}>
                  {columns.map((column) => (
                    <TableCell key={column.key}>
                      <Skeleton className="h-4 w-full max-w-40" />
                    </TableCell>
                  ))}
                </TableRow>
              ))
            : items.map((item, index) => {
                const href = getItemHref?.(item, index);
                const onClick = !href && onItemClick ? () => onItemClick(item, index) : undefined;
                return (
                  <TableRow key={getKey(item, index)} className={cn(href && 'relative', (href || onClick) && 'cursor-pointer')} onClick={onClick}>
                    {columns.map((column) => (
                      <TableCell key={column.key} className={column.className}>
                        {href && column.key === linkKey ? (
                          <Link to={href} className={ROW_LINK_CLASS}>
                            {column.cell(item, index)}
                          </Link>
                        ) : (
                          column.cell(item, index)
                        )}
                      </TableCell>
                    ))}
                  </TableRow>
                );
              })}
        </TableBody>
      </Table>
    </div>
  );
}
