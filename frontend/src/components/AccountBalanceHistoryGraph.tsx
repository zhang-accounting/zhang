import { AccountBalanceHistory } from '../rest-model';
import { LineChart } from '@mantine/charts';
import { groupBy, sortBy } from 'lodash-es';
import BigNumber from 'bignumber.js';

interface Props {
  data?: AccountBalanceHistory;
}

const LINE_COLOR = ['indigo.6', 'blue.6', 'teal.6'];

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
      // @ts-ignore
      let ret: { [commodity: string]: number; date: string } = { date: '' };
      for (let each of it) {
        ret.date = each.date;
        ret[each.balance.commodity] = new BigNumber(each.balance.number).toNumber();
      }
      return ret;
    }),
    (it) => new Date(it.date),
  );
  const series = Object.keys(props.data)
    .sort()
    .map((it, idx) => ({
      name: it,
      color: LINE_COLOR[idx % LINE_COLOR.length],
    }));
  return <LineChart withDots={false} h={250} data={data} dataKey="date" series={series} curveType="linear" />;
}
