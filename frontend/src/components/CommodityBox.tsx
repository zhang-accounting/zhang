import { format } from 'date-fns';
import { ChevronRight } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Badge } from './ui/badge';
import { cn } from '@/lib/utils';
import Amount from './Amount';

export interface CommodityBoxProps {
  name: string;
  total_amount: string;
  operating_currency: boolean;
  prefix?: string | null;
  suffix?: string | null;
  latest_price_amount?: string | null;
  latest_price_commodity?: string | null;
  latest_price_date?: string | null;
}

/** Square tile with the commodity symbol (prefix/suffix) or the first letters of its name. */
export function CommoditySymbol({ name, prefix, suffix, className }: { name: string; prefix?: string | null; suffix?: string | null; className?: string }) {
  const symbol = prefix || suffix || name.slice(0, 3);
  return (
    <span
      aria-hidden
      className={cn(
        'flex size-9 shrink-0 items-center justify-center rounded-lg bg-muted font-semibold text-muted-foreground tabular-nums',
        symbol.length > 2 ? 'text-[11px]' : 'text-sm',
        className,
      )}
    >
      {symbol}
    </span>
  );
}

/** "1 USD = 7.10 CNY" or a muted placeholder when the commodity has no price yet. */
export function CommodityLatestPrice(props: Pick<CommodityBoxProps, 'name' | 'latest_price_amount' | 'latest_price_commodity'>) {
  const { t } = useTranslation();
  if (!props.latest_price_amount || !props.latest_price_commodity) return <span className="text-muted-foreground">{t('commodities.no_price')}</span>;
  return (
    <span className="inline-flex items-center gap-1 tabular-nums">
      <Amount amount={1} currency={props.name} />
      <span className="text-muted-foreground">=</span>
      <Amount amount={props.latest_price_amount} currency={props.latest_price_commodity} />
    </span>
  );
}

/** Mobile card row for a commodity: symbol, name, latest price and holdings. */
export default function CommodityBox(props: CommodityBoxProps) {
  const { t } = useTranslation();
  return (
    <div className="flex items-center gap-3">
      <CommoditySymbol name={props.name} prefix={props.prefix} suffix={props.suffix} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate font-medium">{props.name}</span>
          {props.operating_currency && <Badge variant="secondary">{t('commodities.operating')}</Badge>}
        </div>
        <div className="truncate text-xs">
          <CommodityLatestPrice {...props} />
          {props.latest_price_date && <span className="text-muted-foreground"> · {format(new Date(props.latest_price_date), 'yyyy-MM-dd')}</span>}
        </div>
      </div>
      <div className="shrink-0 text-right">
        <div className="text-sm font-medium tabular-nums">
          <Amount amount={props.total_amount} currency={props.name} />
        </div>
        <div className="text-xs text-muted-foreground">{t('commodities.holdings')}</div>
      </div>
      <ChevronRight className="size-4 shrink-0 text-muted-foreground" />
    </div>
  );
}
