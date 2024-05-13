import BigNumber from 'bignumber.js';
import { format } from 'date-fns';
import { max, min, sortBy } from 'lodash-es';
import { AccountType, StatisticGraphResponse } from '../rest-model';
import { ChartTooltipProps, LineChart } from '@mantine/charts';
import { Paper, Text } from '@mantine/core';
import Amount from './Amount';

function ChartTooltip({
  label,
  payload,
  total_min,
  commodity,
}: ChartTooltipProps & {
  total_min: number;
  commodity: string;
}) {
  if (!payload) return null;

  return (
    <Paper px="md" py="sm" withBorder shadow="md" radius="md">
      <Text fw={500} mb={5}>
        {label}
      </Text>
      {payload.map((item: any) => (
        <Text key={item.name} c={item.color} fz="sm">
          {item.name}: <Amount amount={item.name !== 'total' ? item.value - total_min : item.value} currency={commodity}></Amount>
        </Text>
      ))}
    </Paper>
  );
}

interface Props {
  data: StatisticGraphResponse;
  height: number;
}

export default function ReportGraph(props: Props) {
  const sequencedDate = sortBy(Object.keys(props.data.balances), (date) => new Date(date));

  const labels = sequencedDate.map((date) => format(new Date(date), 'MMM dd'));
  let total_dataset = sequencedDate.map((date) => {
    const target_day = props.data.balances[date];
    return new BigNumber(target_day.calculated.number).toNumber();
  });
  let total_domain = [min(total_dataset) ?? 0, max(total_dataset) ?? 0];

  const income_dataset = sequencedDate
    .map((date) => props.data.changes[date]?.[AccountType.Income])
    .map((amount) => -1 * new BigNumber(amount?.calculated.number ?? '0').toNumber())
    .map((amount) => amount + total_domain[0]);

  const expense_dataset = sequencedDate
    .map((date) => props.data.changes[date]?.[AccountType.Expenses])
    .map((amount) => new BigNumber(amount?.calculated.number ?? '0').toNumber())
    .map((amount) => amount + total_domain[0]);
  const data = labels.map((label, idx) => ({
    date: label,
    total: total_dataset[idx],
    income: income_dataset[idx],
    expense: expense_dataset[idx],
  }));

  return (
    <>
      <LineChart
        h={300}
        data={data}
        dataKey="date"
        withDots={false}
        withYAxis={false}
        withLegend
        tooltipProps={{
          content: ({ label, payload }) => (
            <ChartTooltip total_min={total_domain[0]} commodity={Object.values(props.data.balances)[0].calculated.currency} label={label} payload={payload} />
          ),
        }}
        yAxisProps={{ type: 'number', scale: 'log', domain: total_domain }}
        series={[
          { name: 'total', color: 'violet.6' },
          { name: 'income', color: 'indigo.6' },
          { name: 'expense', color: 'pink' },
        ]}
        connectNulls
        curveType="bump"
      />
    </>
  );
}
