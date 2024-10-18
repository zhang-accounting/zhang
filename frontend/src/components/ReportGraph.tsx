import BigNumber from 'bignumber.js';
import { format } from 'date-fns';
import { max, min, sortBy } from 'lodash-es';
import { AccountType, StatisticGraphResponse } from '../rest-model';
import { Bar, CartesianGrid, ComposedChart, Line, XAxis, YAxis } from 'recharts';
import { ChartConfig, ChartContainer, ChartTooltipContent, ChartTooltip } from './ui/chart';

const chartConfig = {
  total: {
    label: 'Total',
    color: 'hsl(var(--chart-2))',
  },
  income: {
    label: 'Income',
    color: 'var(--color-green-500)',
  },
  expense: {
    label: 'Expense',
    color: 'var(--color-red-500)',
  },
} satisfies ChartConfig;

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
    .map((amount) => -1 * new BigNumber(amount?.calculated.number ?? '0').toNumber());

  const expense_dataset = sequencedDate
    .map((date) => props.data.changes[date]?.[AccountType.Expenses])
    .map((amount) => new BigNumber(amount?.calculated.number ?? '0').toNumber());

  const max_income = Math.max(...income_dataset) + Math.max(...expense_dataset);

  const data = labels.map((label, idx) => ({
    date: label,
    total: total_dataset[idx],
    income: income_dataset[idx],
    expense: expense_dataset[idx],
  }));

  console.log(data, props.height);
  return (
    <>
      <ChartContainer config={chartConfig} className={`h-[300px] w-full`}>
        <ComposedChart accessibilityLayer data={data}>
          <XAxis dataKey="date" tickLine={false} tickMargin={10} axisLine={false} />

          <YAxis hide type="number" domain={total_domain} yAxisId="left" scale="log"
                 padding={{ top: 20, bottom: 20 }} />
          <YAxis hide type="number" domain={[0, max_income]} yAxisId="right" padding={{ top: 20, bottom: 0 }} />

          <ChartTooltip content={<ChartTooltipContent />} />
          <CartesianGrid vertical={false} />

          <Bar dataKey="income" stackId="a" fill="#3b82f6" yAxisId="right" />
          <Bar dataKey="expense" stackId="a" fill="#ef4444" yAxisId="right" />
          <Line type="monotone" dataKey="total" stroke="var(--color-total)" strokeWidth={2} dot={false} activeDot
                yAxisId="left" />
        </ComposedChart>
      </ChartContainer>
    </>
  );
}
