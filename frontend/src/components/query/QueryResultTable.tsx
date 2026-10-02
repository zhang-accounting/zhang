import { QueryAmount, QueryCost, QueryPosition, QueryResult } from '@/api/types';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { isAmount, isInventory, isPosition } from '@/components/query/values';
import { cn } from '@/lib/utils';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

const MAX_RENDERED_ROWS = 1000;
const NUMERIC_TYPES = new Set(['int', 'decimal', 'amount', 'position', 'inventory']);

/**
 * Adds thousands separators to an exact decimal string without converting it to a float.
 * Values that are not plain decimals (e.g. scientific notation) are returned as-is.
 */
function formatDecimal(value: string): string {
  const match = /^([+-]?)(\d+)(\.\d+)?$/.exec(value);
  if (!match) return value;
  const [, sign, integer, fraction = ''] = match;
  return `${sign === '-' ? '-' : ''}${integer.replace(/\B(?=(\d{3})+(?!\d))/g, ',')}${fraction}`;
}

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
    case 'set':
      if (Array.isArray(value)) {
        if (value.length === 0) return <Empty />;
        return (
          <div className="flex flex-wrap gap-1">
            {value.map((item) => (
              <Badge key={String(item)} variant="secondary" className="px-1.5 py-0 font-normal">
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

export default function QueryResultTable({ result }: Props) {
  const { t } = useTranslation();
  const [showAll, setShowAll] = useState(false);
  const truncated = !showAll && result.rows.length > MAX_RENDERED_ROWS;
  const rows = truncated ? result.rows.slice(0, MAX_RENDERED_ROWS) : result.rows;
  const isNumeric = (type: string) => NUMERIC_TYPES.has(type);

  return (
    <div className="flex flex-col gap-2">
      <div className="rounded-md border overflow-x-auto">
        <Table>
          <TableHeader>
            <TableRow>
              {result.columns.map((column, index) => (
                <TableHead key={index} className={cn('whitespace-nowrap', isNumeric(column.type) && 'text-right')}>
                  {column.name}
                  <span className="ml-1 text-xs font-normal text-muted-foreground">{column.type}</span>
                </TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.length === 0 && (
              <TableRow>
                <TableCell colSpan={Math.max(result.columns.length, 1)} className="text-center text-muted-foreground">
                  {t('query.no_rows')}
                </TableCell>
              </TableRow>
            )}
            {rows.map((row, rowIndex) => (
              <TableRow key={rowIndex}>
                {result.columns.map((column, colIndex) => (
                  <TableCell
                    key={colIndex}
                    className={cn(
                      'align-top py-2',
                      isNumeric(column.type) && 'text-right tabular-nums whitespace-nowrap',
                      column.type === 'date' && 'whitespace-nowrap',
                    )}
                  >
                    <QueryCellValue type={column.type} value={row[colIndex]} />
                  </TableCell>
                ))}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
      {truncated && (
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          {t('query.truncated', { shown: MAX_RENDERED_ROWS, total: result.rows.length })}
          <Button variant="link" size="sm" className="h-auto p-0" onClick={() => setShowAll(true)}>
            {t('query.show_all')}
          </Button>
        </div>
      )}
    </div>
  );
}
