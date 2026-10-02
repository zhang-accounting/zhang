import { retrieveSavedQueries } from '@/api/requests';
import { SavedQuery } from '@/api/types';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import { Bookmark, ChevronDown, LoaderCircle } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

interface Props {
  /** Loads the saved query into the editor and runs it. */
  onSelect: (query: string) => void;
}

type State = { status: 'loading' } | { status: 'failed' } | { status: 'loaded'; queries: SavedQuery[] };

export default function SavedQueriesMenu({ onSelect }: Props) {
  const { t } = useTranslation();
  const [state, setState] = useState<State>({ status: 'loading' });

  // fetched on every open, so queries added to the ledger since the page was loaded show up
  const load = async () => {
    setState((prev) => (prev.status === 'loaded' ? prev : { status: 'loading' }));
    try {
      setState({ status: 'loaded', queries: (await retrieveSavedQueries({})).data.data });
    } catch {
      setState({ status: 'failed' });
    }
  };

  return (
    <DropdownMenu onOpenChange={(open) => open && load()}>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm">
          <Bookmark className="mr-2 h-4 w-4" />
          {t('query.saved')}
          <ChevronDown className="ml-2 h-4 w-4" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="max-h-[min(32rem,70vh)] w-[min(28rem,calc(100vw-2rem))] overflow-y-auto">
        {state.status === 'loading' && (
          <DropdownMenuLabel className="flex items-center gap-2 font-normal text-muted-foreground">
            <LoaderCircle className="h-4 w-4 animate-spin" />
            {t('query.saved_loading')}
          </DropdownMenuLabel>
        )}
        {state.status === 'failed' && <DropdownMenuLabel className="font-normal text-destructive">{t('query.saved_load_failed')}</DropdownMenuLabel>}
        {state.status === 'loaded' && state.queries.length === 0 && (
          <div className="flex flex-col gap-1 px-2 py-2 text-sm">
            <span className="font-medium">{t('query.saved_empty')}</span>
            <span className="text-xs text-muted-foreground">{t('query.saved_empty_hint')}</span>
            <code className="break-all text-xs text-muted-foreground">
              2024-01-01 query "expenses" "SELECT account, sum(position) WHERE account ~ '^Expenses' GROUP BY 1"
            </code>
          </div>
        )}
        {state.status === 'loaded' &&
          state.queries.map((saved, index) => (
            <DropdownMenuItem key={`${saved.name}-${index}`} className="flex flex-col items-start gap-1" onSelect={() => onSelect(saved.query)}>
              <div className="flex w-full flex-wrap items-baseline gap-x-2">
                <span className="font-medium">{saved.name}</span>
                {saved.valid === false && <span className="text-xs text-destructive">{t('query.saved_invalid')}</span>}
                <span className="ml-auto text-xs text-muted-foreground">{saved.date}</span>
              </div>
              <code className="line-clamp-2 break-all text-xs text-muted-foreground">{saved.query}</code>
              {saved.valid === false && saved.error && <span className="line-clamp-3 break-all text-xs text-destructive">{saved.error}</span>}
            </DropdownMenuItem>
          ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
