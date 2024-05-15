import { AccountBalanceHistory } from '../rest-model';
import { ChartTooltipProps, LineChart } from '@mantine/charts';
import { groupBy, max, min, sortBy } from 'lodash-es';
import BigNumber from 'bignumber.js';
import { Paper, Text } from '@mantine/core';
import Amount from './Amount';

export function ChartTooltip({ label, payload }: ChartTooltipProps) {
  if (!payload) return null;

  return (
    <Paper px="md" py="sm" withBorder shadow="md" radius="md">
      <Text fw={500} mb={5}>
        {label}
      </Text>
      {payload.map((item: any) => (
        <Text key={item.name} c={item.color} fz="sm">
          {item.name}: <Amount amount={item.value} currency={item.name}></Amount>
        </Text>
      ))}
    </Paper>
  );
}

interface Props {
  data?: AccountBalanceHistory;
}

const LINE_COLOR = ['cyan.6', 'indigo.6', 'blue.6', 'teal.6'];

export function AccountBalanceHistoryGraph(props: Props) {
  if (!props.data) {
    return <div></div>;
  }

  const minAmount =
    min(
      Object.values(props.data)
        .flat()
        .map((it) => it.balance.number)
        .map((it) => new BigNumber(it).toNumber()),
    ) ?? 0;
  const maxAmount =
    max(
      Object.values(props.data)
        .flat()
        .map((it) => it.balance.number)
        .map((it) => new BigNumber(it).toNumber()),
    ) ?? 0;

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
  return (
    <LineChart
      h={250}
      dotProps={{ r: 0, strokeWidth: 1 }}
      data={data}
      dataKey="date"
      series={series}
      yAxisProps={{ type: 'number', scale: 'log', domain: [minAmount, maxAmount] }}
      tooltipProps={{
        content: ({ label, payload }) => <ChartTooltip label={label} payload={payload} />,
      }}
      curveType="linear"
    />
  );
}
