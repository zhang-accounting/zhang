import { useAtomValue } from 'jotai';
import { BookOpenText, ChevronRight, ExternalLink } from 'lucide-react';
import { ReactNode, useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { QueryTableColumnDoc, QueryTableDoc } from '@/api/types';
import { useIsMobile } from '@/components/layout';
import { SheetCloseButton } from '@/components/layout/SheetCloseButton';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible';
import { Input } from '@/components/ui/input';
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle, SheetTrigger } from '@/components/ui/sheet';
import { Spinner } from '@/components/ui/spinner';
import { querySchemaAtom } from '@/states/query';
import { cn } from '@/lib/utils';

const QUERY_DOCS_URL = 'https://zhang-accounting.kilerd.me/reference/query-language/';
/** The table a query without `FROM #table` reads. */
const DEFAULT_TABLE = 'postings';

interface Props {
  /** Inserts `text` at the editor cursor (and focuses the editor), then moves the cursor `cursorBack` characters back. */
  onInsert: (text: string, cursorBack?: number) => void;
  className?: string;
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
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'flex min-h-10 w-full flex-col justify-center gap-1 rounded-md px-2 py-2 text-left outline-none',
        'hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/50',
        className,
      )}
    >
      <span className="flex min-w-0 flex-wrap items-center gap-2">
        <code className="font-mono text-sm font-medium break-all">{title}</code>
        {aside}
      </span>
      {description && <span className="text-xs text-muted-foreground">{description}</span>}
    </button>
  );
}

function ReferenceSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col">
      <h3 className="sticky top-0 z-10 bg-popover px-2 py-1.5 text-xs font-medium tracking-wide text-muted-foreground uppercase">{title}</h3>
      {children}
    </section>
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

/** A table: clicking its name inserts `FROM #name`, the chevron shows or hides its columns. */
function TableGroup({ table, columns, open, onToggle, onInsert }: TableGroupProps) {
  const { t } = useTranslation();
  const columnsId = useId();
  return (
    <Collapsible open={open} onOpenChange={onToggle}>
      <div className="flex items-start">
        <CollapsibleTrigger
          aria-controls={columnsId}
          aria-label={t(open ? 'query.table_collapse' : 'query.table_expand', { name: table.name })}
          className={cn(
            'group/table flex size-10 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none md:mt-1 md:size-8',
            'hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/50',
          )}
        >
          <ChevronRight className="size-4 transition-transform group-data-panel-open/table:rotate-90 motion-reduce:transition-none" />
        </CollapsibleTrigger>
        <ReferenceItem
          className="min-w-0 flex-1"
          title={`#${table.name}`}
          description={table.description}
          aside={
            <>
              {table.name === DEFAULT_TABLE && (
                <Badge variant="secondary" className="font-normal">
                  {t('query.table_default')}
                </Badge>
              )}
              <span className="text-xs text-muted-foreground">{t('query.table_columns', { count: table.columns.length })}</span>
            </>
          }
          onClick={() => onInsert(`FROM #${table.name}`)}
        />
      </div>
      <CollapsibleContent id={columnsId} className="ml-5 border-l pl-2 md:ml-4">
        {columns.map((column) => (
          <ReferenceItem
            key={column.name}
            title={column.name}
            description={column.description}
            aside={
              <Badge variant="outline" className="font-mono font-normal">
                {column.type}
              </Badge>
            }
            onClick={() => onInsert(column.name)}
          />
        ))}
      </CollapsibleContent>
    </Collapsible>
  );
}

/** Tables, columns and functions of the query language (`GET /api/query/schema`): a side sheet on desktop, a bottom sheet on phones. */
export default function QueryReference({ onInsert, className }: Props) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState('');
  const filterRef = useRef<HTMLInputElement>(null);
  // Tables opened without a filter. While filtering, every listed table is open unless it was collapsed during that filter.
  const [openTables, setOpenTables] = useState<Set<string>>(() => new Set([DEFAULT_TABLE]));
  const [collapsedWhileFiltering, setCollapsedWhileFiltering] = useState<Set<string>>(() => new Set());

  // the schema the query editor highlights with too, fetched once
  const schemaState = useAtomValue(querySchemaAtom);
  const loading = schemaState.state === 'loading';
  const error = schemaState.state === 'hasError';
  const schema = schemaState.state === 'hasData' ? schemaState.data : undefined;

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
  ];
  const nothingMatches = schema !== undefined && tables.length === 0 && functions.length === 0;

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

  // The insertion is applied once the sheet has fully closed (focus is no longer trapped), so the editor can take the focus;
  // `returnFocus` stops the sheet from moving the focus back to its trigger in that case.
  const pendingInsert = useRef<{ text: string; cursorBack?: number } | null>(null);
  const returnFocus = useRef(true);
  const onOpenChange = (nextOpen: boolean) => {
    if (nextOpen) returnFocus.current = true;
    setOpen(nextOpen);
  };
  const insert = (text: string, cursorBack?: number) => {
    pendingInsert.current = { text, cursorBack };
    returnFocus.current = false;
    setOpen(false);
  };
  const onOpenChangeComplete = (nextOpen: boolean) => {
    const pending = pendingInsert.current;
    if (nextOpen || pending === null) return;
    pendingInsert.current = null;
    requestAnimationFrame(() => onInsert(pending.text, pending.cursorBack));
  };

  return (
    <Sheet open={open} onOpenChange={onOpenChange} onOpenChangeComplete={onOpenChangeComplete}>
      <SheetTrigger render={<Button variant="outline" className={cn('h-10 md:h-8', className)} />}>
        <BookOpenText />
        {t('query.reference')}
      </SheetTrigger>
      <SheetContent
        side={isMobile ? 'bottom' : 'right'}
        showCloseButton={false}
        initialFocus={isMobile ? true : filterRef}
        finalFocus={() => returnFocus.current}
        className={cn(
          'gap-0',
          isMobile ? 'data-[side=bottom]:h-[85svh] rounded-t-2xl pb-[env(safe-area-inset-bottom)]' : 'w-full data-[side=right]:sm:max-w-md',
        )}
      >
        <SheetHeader className="pr-12">
          <SheetTitle>{t('query.reference')}</SheetTitle>
          <SheetDescription>{t('query.reference_description')}</SheetDescription>
          <a
            href={QUERY_DOCS_URL}
            target="_blank"
            rel="noreferrer"
            className={cn(
              'inline-flex h-10 w-fit items-center gap-1 rounded-sm text-sm text-link underline-offset-4 outline-none md:h-7',
              'hover:underline focus-visible:ring-3 focus-visible:ring-ring/50',
            )}
          >
            {t('query.reference_docs')}
            <ExternalLink className="size-3.5" />
          </a>
        </SheetHeader>
        <div className="px-4 pb-3">
          <Input
            ref={filterRef}
            type="search"
            placeholder={t('query.reference_filter')}
            aria-label={t('query.reference_filter')}
            value={filter}
            onChange={(event) => {
              setFilter(event.currentTarget.value);
              setCollapsedWhileFiltering(new Set());
            }}
          />
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain border-t px-2 pt-2 pb-4">
          {loading && (
            <p className="flex items-center gap-2 px-2 py-2 text-sm text-muted-foreground" role="status">
              <Spinner aria-hidden />
              {t('query.reference_loading')}
            </p>
          )}
          {error && (
            <p className="px-2 py-2 text-sm text-destructive" role="alert">
              {t('query.reference_load_failed')}
            </p>
          )}
          {nothingMatches && <p className="px-2 py-2 text-sm text-muted-foreground">{t('query.reference_no_match')}</p>}
          {schema && (
            <div className="flex flex-col gap-3">
              {tables.length > 0 && (
                <ReferenceSection title={t('query.tables')}>
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
                </ReferenceSection>
              )}
              {functionGroups
                .filter((group) => group.items.length > 0)
                .map((group) => (
                  <ReferenceSection key={group.title} title={group.title}>
                    {group.items.map((func, index) => (
                      <ReferenceItem
                        key={`${func.name}-${index}`}
                        title={func.signature}
                        description={func.description}
                        onClick={() => insert(`${func.name}()`, 1)}
                      />
                    ))}
                  </ReferenceSection>
                ))}
            </div>
          )}
        </div>
        <SheetCloseButton />
      </SheetContent>
    </Sheet>
  );
}
