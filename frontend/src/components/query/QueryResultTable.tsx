import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { QueryAmount, QueryCost, QueryPosition, QueryResult } from '@/api/types';
import { formatDecimal, isAmount, isInventory, isMetas, isPosition } from '@/components/query/values';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { cn } from '@/lib/utils';

const MAX_RENDERED_ROWS = 1000;
const NUMERIC_TYPES = new Set(['int', 'decimal', 'amount', 'position', 'inventory']);
/** Free-text columns wrap (bounded width) instead of stretching the table. */
const WRAPPING_TYPES = new Set(['str', 'set', 'metas']);

function formatAmount(amount: QueryAmount): string {
  return `${formatDecimal(String(amount.number))} ${amount.currency}`;
}

function formatCost(cost: QueryCost): string {
  const parts = [formatAmount(cost)];
  if (cost.date) parts.push(cost.date);
  if (cost.label) parts.push(`"${cost.label}"`);
  return `{${parts.join(', ')}}`;
}

function formatPosition(position: QueryPosition): string {
  const units = formatAmount(position.units);
  return position.cost ? `${units} ${formatCost(position.cost)}` : units;
}

function Empty() {
  return <span className="text-muted-foreground">—</span>;
}

function QueryCellValue({ type, value }: { type: string; value: unknown }) {
  if (value === null || value === undefined) return <Empty />;

  switch (type) {
    case 'decimal':
      return <>{formatDecimal(String(value))}</>;
    case 'amount':
      if (isAmount(value)) return <>{formatAmount(value)}</>;
      break;
    case 'position':
      if (isPosition(value)) return <>{formatPosition(value)}</>;
      break;
    case 'inventory':
      if (isInventory(value)) {
        if (value.positions.length === 0) return <Empty />;
        return (
          <div className="flex flex-col">
            {value.positions.map((position, index) => (
              <span key={index}>{formatPosition(position)}</span>
            ))}
          </div>
        );
      }
      break;
    case 'metas':
      if (isMetas(value)) {
        if (value.length === 0) return <Empty />;
        return (
          <div className="flex flex-col">
            {value.map((meta, index) => (
              <span key={index}>
                <span className="text-muted-foreground">{meta.key}:</span> {meta.value}
              </span>
            ))}
          </div>
        );
      }
      break;
    case 'set':
      if (Array.isArray(value)) {
        if (value.length === 0) return <Empty />;
        return (
          <div className="flex flex-wrap gap-1">
            {value.map((item) => (
              <Badge key={String(item)} variant="secondary" className="font-normal">
                {String(item)}
              </Badge>
            ))}
          </div>
        );
      }
      break;
    default:
      break;
  }

  if (typeof value === 'object') return <span className="text-muted-foreground">{JSON.stringify(value)}</span>;
  return <>{String(value)}</>;
}

interface Props {
  result: QueryResult;
}

/**
 * Typed result grid for the results card: scrolls inside its own container (both axes) with a sticky header, so the page
 * never overflows.
 */
export default function QueryResultTable({ result }: Props) {
  const { t } = useTranslation();
  const [showAll, setShowAll] = useState(false);
  const truncated = !showAll && result.rows.length > MAX_RENDERED_ROWS;
  const rows = truncated ? result.rows.slice(0, MAX_RENDERED_ROWS) : result.rows;
  const isNumeric = (type: string) => NUMERIC_TYPES.has(type);

  return (
    <>
      <div
        className={cn(
          '[&>[data-slot=table-container]]:max-h-[65svh] [&>[data-slot=table-container]]:overflow-auto',
          '[&>[data-slot=table-container]]:overscroll-x-contain',
        )}
      >
        <Table className="text-[13px]">
          <TableHeader className="sticky top-0 z-10 bg-card shadow-[0_1px_0_var(--border)]">
            <TableRow className="hover:bg-transparent">
              {result.columns.map((column, index) => (
                <TableHead key={index} className={cn('px-3 first:pl-4 last:pr-4', isNumeric(column.type) && 'text-right')}>
                  <span className="font-mono text-xs">{column.name}</span>
                  <span className="ml-1.5 text-[11px] font-normal text-muted-foreground">{column.type}</span>
                </TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.length === 0 && (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={Math.max(result.columns.length, 1)} className="py-8 text-center text-muted-foreground">
                  {t('query.no_rows')}
                </TableCell>
              </TableRow>
            )}
            {rows.map((row, rowIndex) => (
              <TableRow key={rowIndex}>
                {result.columns.map((column, colIndex) => (
                  <TableCell key={colIndex} className={cn('px-3 py-2 align-top first:pl-4 last:pr-4', isNumeric(column.type) && 'text-right tabular-nums')}>
                    {WRAPPING_TYPES.has(column.type) ? (
                      <div className="max-w-80 min-w-24 break-words whitespace-normal">
                        <QueryCellValue type={column.type} value={row[colIndex]} />
                      </div>
                    ) : (
                      <QueryCellValue type={column.type} value={row[colIndex]} />
                    )}
                  </TableCell>
                ))}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
      {truncated && (
        <div className="flex flex-wrap items-center gap-x-2 border-t px-4 py-2 text-sm text-muted-foreground">
          {t('query.truncated', { shown: MAX_RENDERED_ROWS, total: result.rows.length })}
          <Button variant="link" size="sm" className="h-10 px-0 text-link md:h-7" onClick={() => setShowAll(true)}>
            {t('query.show_all')}
          </Button>
        </div>
      )}
    </>
  );
}
