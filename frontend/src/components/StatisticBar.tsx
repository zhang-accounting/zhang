import { retrieveStatisticSummary } from '@/api/requests.ts';
import StatisticBox from './StatisticBox';
import { useAsync } from 'react-use';

export default function StatisticBar() {
  const now = new Date();
  const beginning_time = new Date(now.getFullYear(), now.getMonth() - 1, now.getDate(), 0, 0, 1);
  const end_time = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 23, 59, 59);

  const { value: data, loading, error } = useAsync(async () => {
    const res =  await retrieveStatisticSummary({ from: beginning_time.toISOString(), to: end_time.toISOString() });
    return res.data.data;
  });

  if (error) return <div>failed to load</div>;
  if (loading || !data) return <>loading</>;

  return (
    <>
      <div className="grid gap-4 md:grid-cols-2 md:gap-4 lg:grid-cols-4">
        <StatisticBox
          text={'ASSET_BALANCE'}
          amount={data.balance.calculated.number}
          currency={data.balance.calculated.currency}
          hint={'include assets and liabilities'}
        />
        <StatisticBox text={'LIABILITY'} amount={data.liability.calculated.number} currency={data.liability.calculated.currency} negative />
        <StatisticBox text={'CURRENT_MONTH_INCOME'} amount={data.income.calculated.number} currency={data.income.calculated.currency} negative />
        <StatisticBox text={'CURRENT_MONTH_EXPENSE'} amount={data.expense.calculated.number} currency={data.expense.calculated.currency} />
      </div>
    </>
  );
}
