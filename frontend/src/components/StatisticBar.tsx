import { ArrowDownLeft, ArrowUpRight, CreditCard, Landmark } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useAsync } from 'react-use';
import { retrieveStatisticSummary } from '@/api/requests';
import { cn } from '@/lib/utils';
import StatisticBox from './StatisticBox';

interface Props {
  from: Date;
  to: Date;
  /** Hint under the income card, e.g. "Last 30 days". */
  periodLabel?: string;
  className?: string;
}

/** Dashboard KPIs for a period: 2 columns on mobile, 4 from lg. */
export default function StatisticBar({ from, to, periodLabel, className }: Props) {
  const { t } = useTranslation();
  const {
    value: data,
    loading,
    error,
  } = useAsync(async () => {
    const res = await retrieveStatisticSummary({ from: from.toISOString(), to: to.toISOString() });
    return res.data.data;
  }, [from.getTime(), to.getTime()]);

  const isLoading = loading || (!data && !error);

  return (
    <div className={cn('grid grid-cols-2 gap-3 md:gap-4 lg:grid-cols-4', className)}>
      <StatisticBox
        text="ASSET_BALANCE"
        icon={Landmark}
        loading={isLoading}
        amount={data?.balance.calculated.number ?? '0'}
        currency={data?.balance.calculated.commodity ?? ''}
        hint={error ? t('ledger.common.load_failed') : t('ledger.home.net_worth_hint')}
      />
      <StatisticBox
        text="LIABILITY"
        icon={CreditCard}
        loading={isLoading}
        amount={data?.liability.calculated.number ?? '0'}
        currency={data?.liability.calculated.commodity ?? ''}
        negative
        hint={t('ledger.home.liability_hint')}
      />
      <StatisticBox
        text="ledger.chart.income"
        icon={ArrowDownLeft}
        loading={isLoading}
        amount={data?.income.calculated.number ?? '0'}
        currency={data?.income.calculated.commodity ?? ''}
        negative
        hint={periodLabel}
      />
      <StatisticBox
        text="ledger.chart.expenses"
        icon={ArrowUpRight}
        loading={isLoading}
        amount={data?.expense.calculated.number ?? '0'}
        currency={data?.expense.calculated.commodity ?? ''}
        hint={t('ledger.home.transactions', { count: data?.transaction_number ?? 0 })}
      />
    </div>
  );
}
