import { BookOpenText, ExternalLink } from 'lucide-react';
import { ReactNode, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useAsync } from 'react-use';
import { retrieveQuerySchema } from '@/api/requests';
import { useIsMobile } from '@/components/layout';
import { SheetCloseButton } from '@/components/layout/SheetCloseButton';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle, SheetTrigger } from '@/components/ui/sheet';
import { Spinner } from '@/components/ui/spinner';
import { cn } from '@/lib/utils';

const QUERY_DOCS_URL = 'https://zhang-accounting.kilerd.me/';

interface Props {
  /** Inserts `text` at the editor cursor (and focuses the editor), then moves the cursor `cursorBack` characters back. */
  onInsert: (text: string, cursorBack?: number) => void;
  className?: string;
}

function ReferenceItem({ title, description, aside, onClick }: { title: string; description: string; aside?: ReactNode; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'flex min-h-10 w-full flex-col justify-center gap-1 rounded-md px-2 py-2 text-left outline-none',
        'hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/50',
      )}
    >
      <span className="flex min-w-0 items-center gap-2">
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

/** Columns and functions of the query language (`GET /api/query/schema`): a side sheet on desktop, a bottom sheet on phones. */
export default function QueryReference({ onInsert, className }: Props) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState('');
  const filterRef = useRef<HTMLInputElement>(null);

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
  const columns = (schema?.columns ?? []).filter((column) => matches(column.name, column.description));
  const functions = (schema?.functions ?? []).filter((func) => matches(func.name, func.signature, func.description));
  const functionGroups = [
    { title: t('query.aggregates'), items: functions.filter((func) => func.aggregate) },
    { title: t('query.functions'), items: functions.filter((func) => !func.aggregate) },
  ];
  const nothingMatches = schema !== undefined && columns.length === 0 && functions.length === 0;

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
            onChange={(event) => setFilter(event.currentTarget.value)}
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
              {columns.length > 0 && (
                <ReferenceSection title={t('query.columns')}>
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
                      onClick={() => insert(column.name)}
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
