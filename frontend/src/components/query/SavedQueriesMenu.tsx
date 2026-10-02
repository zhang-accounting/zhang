import { Bookmark, ChevronDown } from 'lucide-react';
import { useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { retrieveSavedQueries } from '@/api/requests';
import { SavedQuery } from '@/api/types';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import { Spinner } from '@/components/ui/spinner';
import { cn } from '@/lib/utils';

interface Props {
  /** Loads the saved query into the editor and runs it. */
  onSelect: (query: string) => void;
  className?: string;
}

type State = { status: 'loading' } | { status: 'failed' } | { status: 'loaded'; queries: SavedQuery[] };

const SAVED_QUERY_SAMPLE = `2024-01-01 query "expenses" "SELECT account, sum(position) WHERE account ~ '^Expenses' GROUP BY 1"`;

/** `query` directives of the ledger (`GET /api/query/saved`), refetched on every open. */
export default function SavedQueriesMenu({ onSelect, className }: Props) {
  const { t } = useTranslation();
  const [state, setState] = useState<State>({ status: 'loading' });
  const latestRequest = useRef(0);

  // fetched on every open, so queries added to the ledger since the page was loaded show up
  const load = async () => {
    const request = ++latestRequest.current;
    setState((prev) => (prev.status === 'loaded' ? prev : { status: 'loading' }));
    let next: State;
    try {
      next = { status: 'loaded', queries: (await retrieveSavedQueries({})).data.data };
    } catch {
      next = { status: 'failed' };
    }
    // a slower, older response must not overwrite the one of a later open
    if (request === latestRequest.current) setState(next);
  };

  return (
    <DropdownMenu onOpenChange={(open) => open && load()}>
      <DropdownMenuTrigger render={<Button variant="outline" className={cn('h-10 md:h-8', className)} />}>
        <Bookmark />
        {t('query.saved')}
        <ChevronDown className="text-muted-foreground" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="max-h-[min(32rem,70svh)] w-[min(28rem,calc(100vw-2rem))] overscroll-contain">
        {state.status === 'loading' && (
          <div role="status" className="flex items-center gap-2 px-2 py-2 text-sm text-muted-foreground">
            <Spinner aria-hidden />
            {t('query.saved_loading')}
          </div>
        )}
        {state.status === 'failed' && (
          <div role="alert" className="px-2 py-2 text-sm text-destructive">
            {t('query.saved_load_failed')}
          </div>
        )}
        {state.status === 'loaded' && state.queries.length === 0 && (
          <div className="flex flex-col gap-1 px-2 py-2 text-sm">
            <span className="font-medium">{t('query.saved_empty')}</span>
            <span className="text-xs text-muted-foreground">{t('query.saved_empty_hint')}</span>
            <code className="rounded-md bg-muted px-2 py-1.5 font-mono text-xs break-all text-muted-foreground">{SAVED_QUERY_SAMPLE}</code>
          </div>
        )}
        {state.status === 'loaded' &&
          state.queries.map((saved, index) => (
            <DropdownMenuItem key={`${saved.name}-${index}`} className="flex min-h-10 flex-col items-start gap-1 py-2" onClick={() => onSelect(saved.query)}>
              <span className="flex w-full min-w-0 flex-wrap items-baseline gap-x-2">
                <span className="font-medium break-all">{saved.name}</span>
                {saved.valid === false && <span className="text-xs text-destructive">{t('query.saved_invalid')}</span>}
                <span className="ml-auto text-xs text-muted-foreground tabular-nums">{saved.date}</span>
              </span>
              <code className="line-clamp-2 font-mono text-xs break-all text-muted-foreground">{saved.query}</code>
              {saved.valid === false && saved.error && <span className="line-clamp-3 text-xs break-all text-destructive">{saved.error}</span>}
            </DropdownMenuItem>
          ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
