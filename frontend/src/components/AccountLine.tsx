import { ChevronRight } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router';
import { useLocalStorage } from '@/hooks/use-local-storage';
import { cn } from '@/lib/utils';
import { heldCommodities, TreeTotal } from '../utils/account-totals';
import AccountTrie from '../utils/AccountTrie';
import { accountCollapseKey } from './layout/account-tree';
import Amount from './Amount';
import { Badge } from './ui/badge';

interface Props {
  data: AccountTrie;
  /** Depth in the tree (0 = first level below the account type). */
  spacing: number;
  /** Show every descendant regardless of the stored collapse state (used while searching). */
  forceExpand?: boolean;
  /** The value of every node of the whole tree, by path (`treeTotals`): the same whatever the page filters. */
  totals: Map<string, TreeTotal>;
}

const FOCUS_RING = 'outline-none focus-visible:ring-3 focus-visible:ring-ring/50';
const TOGGLE_CLASS = cn('flex size-10 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-muted md:size-6', FOCUS_RING);
const LINK_CLASS = cn('flex min-h-10 min-w-0 flex-1 items-center rounded-md hover:underline hover:underline-offset-4 md:min-h-8', FOCUS_RING);

/** One row of the account tree (div based, so it works as a card row on mobile and a dense list on desktop). */
export default function AccountLine({ data, spacing, forceExpand = false, totals }: Props) {
  const { t } = useTranslation();
  const [isShow, setCollapse] = useLocalStorage({ key: accountCollapseKey(data.path), defaultValue: false });
  const hasChildren = Object.keys(data.children).length > 0;
  const expanded = hasChildren && (forceExpand || isShow);
  const total = totals.get(data.path);
  const commodities = heldCommodities(total);
  const haveMultipleCommodity = commodities.length > 1;
  const account = data.val;
  const isClosed = account?.status === 'Close';
  const label = account?.alias ?? data.word;

  const name = (
    <span className="flex min-w-0 flex-col">
      <span className="flex min-w-0 items-center gap-2">
        <span className={cn('truncate text-sm', hasChildren && 'font-medium', isClosed && 'text-muted-foreground')}>{label}</span>
        {isClosed && (
          <Badge variant="outline" className="font-normal text-muted-foreground">
            {t('ledger.accounts.closed')}
          </Badge>
        )}
      </span>
      {account?.alias && <span className="truncate text-xs text-muted-foreground">{account.name}</span>}
    </span>
  );

  return (
    <>
      <div
        className="flex min-h-12 items-center gap-1 border-t pr-4 transition-colors hover:bg-muted/40 md:min-h-10"
        style={{ paddingLeft: `${8 + spacing * 16}px` }}
      >
        {hasChildren ? (
          <button
            type="button"
            className={TOGGLE_CLASS}
            aria-label={expanded ? t('ledger.accounts.collapse') : t('ledger.accounts.expand')}
            aria-expanded={expanded}
            disabled={forceExpand}
            onClick={() => setCollapse(!isShow)}
          >
            <ChevronRight className={cn('size-4 transition-transform', expanded && 'rotate-90')} />
          </button>
        ) : (
          <span className="size-10 shrink-0 md:size-6" />
        )}
        {account ? (
          <Link to={`/accounts/${account.name}`} className={LINK_CLASS}>
            {name}
          </Link>
        ) : (
          <button
            type="button"
            className="flex min-h-10 min-w-0 flex-1 items-center text-left md:min-h-8"
            disabled={forceExpand}
            onClick={() => setCollapse(!isShow)}
          >
            {name}
          </button>
        )}
        <div className={cn('flex shrink-0 flex-col items-end text-sm', !data.isLeaf && 'text-muted-foreground', isClosed && 'text-muted-foreground')}>
          <span className="flex items-baseline gap-1">
            {haveMultipleCommodity && <span aria-hidden>≈</span>}
            {total && <Amount amount={total.number} currency={total.commodity} />}
          </span>
          {haveMultipleCommodity &&
            commodities.map(([commodity, value]) => <Amount key={commodity} className="text-xs text-muted-foreground" amount={value} currency={commodity} />)}
        </div>
      </div>
      {expanded &&
        Object.keys(data.children)
          .sort()
          .map((child) => (
            <AccountLine key={data.children[child].path} data={data.children[child]} spacing={spacing + 1} forceExpand={forceExpand} totals={totals} />
          ))}
    </>
  );
}
