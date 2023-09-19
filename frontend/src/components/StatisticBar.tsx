import { SimpleGrid } from '@mantine/core';
import StatisticBox from './StatisticBox';
import useSWR from 'swr';
import { fetcher } from '../index';
import { StatisticResponse } from '../rest-model';

export default function StatisticBar() {
  const now = new Date();
  const beginning_time = new Date(now.getFullYear(), now.getMonth() - 1, now.getDate(), 0, 0, 1);
  const end_time = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 23, 59, 59);

  const { data, error } = useSWR<StatisticResponse>(`/api/statistic/summary?from=${beginning_time.toISOString()}&to=${end_time.toISOString()}`, fetcher);

  if (error) return <div>failed to load</div>;
  if (!data) return <>loading</>;

  return (
    <SimpleGrid cols={6}>
      <StatisticBox
        text={'ASSET_BLANACE'}
        amount={data.balance.calculated.number}
        currency={data.balance.calculated.commodity}
        hint={'include assets and liabilities'}
      />
      <StatisticBox text={'LIABILITY'} amount={data.liability.calculated.number} currency={data.liability.calculated.commodity} negetive />
      <StatisticBox text={'CURRENT_MONTH_INCOME'} amount={data.income.calculated.number} currency={data.income.calculated.commodity} negetive />
      <StatisticBox text={'CURRENT_MONTH_EXPENSE'} amount={data.expense.calculated.number} currency={data.expense.calculated.commodity} />
    </SimpleGrid>
  );
}
