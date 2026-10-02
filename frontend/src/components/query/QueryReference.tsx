import { retrieveQuerySchema } from '@/api/requests';
import { QueryTableColumnDoc, QueryTableDoc } from '@/api/types';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle, SheetTrigger } from '@/components/ui/sheet';
import { cn } from '@/lib/utils';
import { BookOpenText, ChevronRight, ExternalLink } from 'lucide-react';
import { ReactNode, useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useAsync } from 'react-use';

const QUERY_DOCS_URL = 'https://zhang-accounting.kilerd.me/';
/** The table a query without `FROM #table` reads. */
const DEFAULT_TABLE = 'postings';

interface Props {
  /** Inserts `text` at the editor cursor (and focuses the editor), then moves the cursor `cursorBack` characters back. */
  onInsert: (text: string, cursorBack?: number) => void;
}

function ReferenceItem({
  title,
  description,
  aside,
  className,
  onClick,
}: {
  title: string;
  description: string;
  aside?: ReactNode;
  className?: string;
  onClick: () => void;
}) {
  return (
    <button type="button" onClick={onClick} className={cn('flex w-full flex-col gap-1 rounded-md px-2 py-2 text-left hover:bg-muted', className)}>
      <div className="flex flex-wrap items-center gap-2">
        <code className="break-all text-sm font-medium">{title}</code>
        {aside}
      </div>
      {description && <p className="text-xs text-muted-foreground">{description}</p>}
    </button>
  );
}

interface TableGroupProps {
  table: QueryTableDoc;
  /** the columns to list: all of them, or only those matching the filter */
  columns: QueryTableColumnDoc[];
  open: boolean;
  onToggle: () => void;
  onInsert: (text: string) => void;
}

function TableGroup({ table, columns, open, onToggle, onInsert }: TableGroupProps) {
  const { t } = useTranslation();
  const columnsId = useId();
  return (
    <div>
      <div className="flex items-start">
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={open}
          aria-controls={columnsId}
          aria-label={t(open ? 'query.table_collapse' : 'query.table_expand', { name: table.name })}
          className="mt-1 flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-muted"
        >
          <ChevronRight className={cn('h-4 w-4 transition-transform', open && 'rotate-90')} />
        </button>
        <ReferenceItem
          className="min-w-0 flex-1"
          title={`#${table.name}`}
          description={table.description}
          aside={
            <>
              {table.name === DEFAULT_TABLE && (
                <Badge variant="secondary" className="px-1.5 py-0 font-normal">
                  {t('query.table_default')}
                </Badge>
              )}
              <span className="text-xs text-muted-foreground">{t('query.table_columns', { count: table.columns.length })}</span>
            </>
          }
          onClick={() => onInsert(`FROM #${table.name}`)}
        />
      </div>
      {open && (
        <div id={columnsId} className="ml-4 border-l pl-3">
          {columns.map((column) => (
            <ReferenceItem
              key={column.name}
              title={column.name}
              description={column.description}
              aside={
                <Badge variant="outline" className="px-1.5 py-0 font-normal">
                  {column.type}
                </Badge>
              }
              onClick={() => onInsert(column.name)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

export default function QueryReference({ onInsert }: Props) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState('');
  // Tables opened without a filter. While filtering, every listed table is open unless it was collapsed during that filter.
  const [openTables, setOpenTables] = useState<Set<string>>(() => new Set([DEFAULT_TABLE]));
  const [collapsedWhileFiltering, setCollapsedWhileFiltering] = useState<Set<string>>(() => new Set());

  const {
    loading,
    error,
    value: schema,
  } = useAsync(async () => {
    const res = await retrieveQuerySchema({});
    return res.data.data;
  }, []);

  const keyword = filter.trim().toLowerCase();
  const matches = (...texts: string[]) => keyword === '' || texts.some((text) => text.toLowerCase().includes(keyword));
  // the tables come default table first; a table matching the filter lists all its columns, any other table only its
  // matching columns
  const tables = (schema?.tables ?? []).flatMap((table) => {
    const tableMatches = matches(table.name, `#${table.name}`, table.description);
    const columns = tableMatches ? table.columns : table.columns.filter((column) => matches(column.name, column.description));
    return tableMatches || columns.length > 0 ? [{ table, columns }] : [];
  });
  const functions = (schema?.functions ?? []).filter((func) => matches(func.name, func.signature, func.description));
  const functionGroups = [
    { title: t('query.aggregates'), items: functions.filter((func) => func.aggregate) },
    { title: t('query.functions'), items: functions.filter((func) => !func.aggregate) },
  ].filter((group) => group.items.length > 0);

  const isOpen = (name: string) => (keyword === '' ? openTables.has(name) : !collapsedWhileFiltering.has(name));
  const toggle = (name: string) => {
    const flip = (names: Set<string>) => {
      const next = new Set(names);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    };
    if (keyword === '') setOpenTables(flip);
    else setCollapsedWhileFiltering(flip);
  };

  // The insertion is applied once the sheet has closed, so the editor can take the focus back from the dialog.
  const pendingInsert = useRef<{ text: string; cursorBack?: number } | null>(null);
  const insert = (text: string, cursorBack?: number) => {
    pendingInsert.current = { text, cursorBack };
    setOpen(false);
  };
  const onCloseAutoFocus = (event: Event) => {
    const pending = pendingInsert.current;
    if (pending === null) return;
    pendingInsert.current = null;
    event.preventDefault();
    onInsert(pending.text, pending.cursorBack);
  };

  return (
    <Sheet open={open} onOpenChange={setOpen}>
      <SheetTrigger asChild>
        <Button variant="outline" size="sm">
          <BookOpenText className="mr-2 h-4 w-4" />
          {t('query.reference')}
        </Button>
      </SheetTrigger>
      <SheetContent className="flex w-full flex-col sm:max-w-md" onCloseAutoFocus={onCloseAutoFocus}>
        <SheetHeader>
          <SheetTitle>{t('query.reference')}</SheetTitle>
          <SheetDescription>{t('query.reference_description')}</SheetDescription>
          <a
            href={QUERY_DOCS_URL}
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-1 text-sm text-primary underline-offset-4 hover:underline"
          >
            {t('query.reference_docs')}
            <ExternalLink className="h-3 w-3" />
          </a>
        </SheetHeader>
        <Input
          placeholder={t('query.reference_filter')}
          value={filter}
          onChange={(event) => {
            setFilter(event.currentTarget.value);
            setCollapsedWhileFiltering(new Set());
          }}
        />
        <div className="-mx-2 flex-1 overflow-y-auto">
          {loading && <p className="px-2 text-sm text-muted-foreground">{t('query.reference_loading')}</p>}
          {error && <p className="px-2 text-sm text-destructive">{t('query.reference_load_failed')}</p>}
          {schema && (
            <div className="flex flex-col gap-4">
              {tables.length > 0 && (
                <section>
                  <h3 className="px-2 text-sm font-semibold">{t('query.tables')}</h3>
                  <p className="px-2 pb-1 text-xs text-muted-foreground">{t('query.tables_hint')}</p>
                  {tables.map(({ table, columns }) => (
                    <TableGroup
                      key={table.name}
                      table={table}
                      columns={columns}
                      open={isOpen(table.name)}
                      onToggle={() => toggle(table.name)}
                      onInsert={insert}
                    />
                  ))}
                </section>
              )}
              {functionGroups.map((group) => (
                <section key={group.title}>
                  <h3 className="px-2 pb-1 text-sm font-semibold">{group.title}</h3>
                  {group.items.map((func, index) => (
                    <ReferenceItem
                      key={`${func.name}-${index}`}
                      title={func.signature}
                      description={func.description}
                      onClick={() => insert(`${func.name}()`, 1)}
                    />
                  ))}
                </section>
              ))}
              {tables.length === 0 && functionGroups.length === 0 && <p className="px-2 text-sm text-muted-foreground">{t('query.reference_no_matches')}</p>}
            </div>
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}
