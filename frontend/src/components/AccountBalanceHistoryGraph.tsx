import { AccountBalanceHistory } from '../rest-model';
import { LineChart } from '@mantine/charts';
import { groupBy, sortBy } from 'lodash-es';
import BigNumber from 'bignumber.js';

interface Props {
  data?: AccountBalanceHistory;
}

const LINE_COLOR = ['cyan.6', 'indigo.6', 'blue.6', 'teal.6'];

export function AccountBalanceHistoryGraph(props: Props) {
  if (!props.data) {
    return <div></div>;
  }

  const dataByDate = groupBy(
    Object.values(props.data).flatMap((it) => it),
    (it) => it.date,
  );

  const data = sortBy(
    Object.values(dataByDate).map((it) => {
      return it.reduce(
        (acc, each) => {
          acc.date = each.date;
          acc[each.balance.commodity] = new BigNumber(each.balance.number).toNumber();
          return acc;
        },
        {} as Record<string, string | number>,
      );
    }),
    (it) => new Date(it.date),
  );
  const series = Object.keys(props.data)
    .sort()
    .map((it, idx) => ({
      name: it,
      color: LINE_COLOR[idx % LINE_COLOR.length],
    }));
  return <LineChart h={250} dotProps={{ r: 0, strokeWidth: 1 }} data={data} dataKey="date" series={series} curveType="linear" />;
}
