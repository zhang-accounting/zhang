import { SimpleGrid } from '@mantine/core';
import StatisticBox from './StatisticBox';
import useSWR from 'swr';
import { fetcher } from '../index';
import { CurrentStatisticResponse } from '../rest-model';

export default function StatisticBar() {
  const { data, error } = useSWR<CurrentStatisticResponse>('/api/statistic/current', fetcher);

  if (error) return <div>failed to load</div>;
  if (!data) return <>loading</>;

  return (
    <SimpleGrid cols={6} >
      <StatisticBox text={'ASSET_BLANACE'} amount={data.balance.calculated.number} currency={data.balance.calculated.commodity} hint={"include assets and liabilities"} />
      <StatisticBox text={'LIABILITY'} amount={data.liability.calculated.number} currency={data.liability.calculated.commodity} negetive />
      <StatisticBox text={'CURRENT_MONTH_INCOME'} amount={data.income.number} currency={data.income.commodity} negetive />
      <StatisticBox text={'CURRENT_MONTH_EXPENSE'} amount={data.expense.number} currency={data.expense.commodity} />
    </SimpleGrid>
  );
}
