import { retrieveQuerySchema } from '@/api/requests';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle, SheetTrigger } from '@/components/ui/sheet';
import { BookOpenText, ExternalLink } from 'lucide-react';
import { ReactNode, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useAsync } from 'react-use';

const QUERY_DOCS_URL = 'https://zhang-accounting.kilerd.me/';

interface Props {
  /** Inserts `text` at the editor cursor (and focuses the editor), then moves the cursor `cursorBack` characters back. */
  onInsert: (text: string, cursorBack?: number) => void;
}

function ReferenceItem({ title, description, aside, onClick }: { title: string; description: string; aside?: ReactNode; onClick: () => void }) {
  return (
    <button type="button" onClick={onClick} className="flex w-full flex-col gap-1 rounded-md px-2 py-2 text-left hover:bg-muted">
      <div className="flex items-center gap-2">
        <code className="break-all text-sm font-medium">{title}</code>
        {aside}
      </div>
      {description && <p className="text-xs text-muted-foreground">{description}</p>}
    </button>
  );
}

export default function QueryReference({ onInsert }: Props) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState('');

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
        <Input placeholder={t('query.reference_filter')} value={filter} onChange={(event) => setFilter(event.currentTarget.value)} />
        <div className="-mx-2 flex-1 overflow-y-auto">
          {loading && <p className="px-2 text-sm text-muted-foreground">{t('query.reference_loading')}</p>}
          {error && <p className="px-2 text-sm text-destructive">{t('query.reference_load_failed')}</p>}
          {schema && (
            <div className="flex flex-col gap-4">
              <section>
                <h3 className="px-2 pb-1 text-sm font-semibold">{t('query.columns')}</h3>
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
                    onClick={() => insert(column.name)}
                  />
                ))}
              </section>
              <section>
                <h3 className="px-2 pb-1 text-sm font-semibold">{t('query.functions')}</h3>
                {functions.map((func, index) => (
                  <ReferenceItem
                    key={`${func.name}-${index}`}
                    title={func.signature}
                    description={func.description}
                    onClick={() => insert(`${func.name}()`, 1)}
                  />
                ))}
              </section>
            </div>
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}
