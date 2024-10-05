import BigNumber from 'bignumber.js';
import { format } from 'date-fns';
import { max, min, sortBy } from 'lodash-es';
import { AccountType, StatisticGraphResponse } from '../rest-model';
import { Bar, CartesianGrid, ComposedChart, Line, XAxis } from 'recharts';
import { ChartConfig, ChartContainer, ChartTooltipContent, ChartTooltip } from './ui/chart';


const chartConfig = {
  total: {
    label: "Total",
    color: "#ff7300",
  },
  income: {
    label: "Income",
    color: "#2563eb",
  },
  expense: {
    label: "Expense",
    color: "#60a5fa",
  },
 
  } satisfies ChartConfig


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
  let total_domain = [(min(total_dataset) ?? 0) * 0.999, (max(total_dataset) ?? 0) * 1.001];

  const income_dataset = sequencedDate
    .map((date) => props.data.changes[date]?.[AccountType.Income])
    .map((amount) => -1 * new BigNumber(amount?.calculated.number ?? '0').toNumber());

  const expense_dataset = sequencedDate
    .map((date) => props.data.changes[date]?.[AccountType.Expenses])
    .map((amount) => new BigNumber(amount?.calculated.number ?? '0').toNumber());

  const data = labels.map((label, idx) => ({
    date: label,
    total: total_dataset[idx],
    income: income_dataset[idx],
    expense: expense_dataset[idx],
  }));

  console.log(data);
  return (
    <>
      <ChartContainer config={chartConfig} className="min-h-[200px] w-full">
        <ComposedChart accessibilityLayer data={data}>
          <XAxis
            dataKey="date"
            tickLine={false}
            tickMargin={10}
            axisLine={false}
          />
          <ChartTooltip content={<ChartTooltipContent />} />
          <CartesianGrid vertical={false} />
          
          <Bar dataKey="income" stackId="a" fill="var(--color-income)" yAxisId="right" radius={4} />
          <Bar dataKey="expense" stackId="a" fill="var(--color-expense)" yAxisId="right" radius={4} />
          <Line type="monotone" dataKey="total" stroke="#ff7300" yAxisId="left" />
        </ComposedChart>
      </ChartContainer>

      {/* todo log y axis */}
      {/* todo chart color */}
    </>
  );
}
