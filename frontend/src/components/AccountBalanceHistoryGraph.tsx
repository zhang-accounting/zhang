import { AccountBalanceHistory } from '../rest-model';
import { groupBy, max, min, sortBy } from 'lodash-es';
import BigNumber from 'bignumber.js';
import { ChartConfig, ChartContainer, ChartTooltip, ChartTooltipContent } from './ui/chart';
import { CartesianGrid, Line, LineChart, XAxis, YAxis } from 'recharts';

const COLOR_SET = ['hsl(var(--chart-1))', 'hsl(var(--chart-2))', 'hsl(var(--chart-3))', 'hsl(var(--chart-4))', 'hsl(var(--chart-5))'];

function buildChartConfig(series: { name: string; color: string }[]): ChartConfig {
  return series.reduce(
    (acc, current, idx) => ({
      ...acc,
      [current.name]: {
        label: current.name,
        color: COLOR_SET[idx % COLOR_SET.length],
      },
    }),
    {},
  ) satisfies ChartConfig;
}

interface Props {
  data?: AccountBalanceHistory;
}

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
      color: COLOR_SET[idx % COLOR_SET.length],
    }));

  console.log('account graph', data, series);
  return (
    <ChartContainer config={buildChartConfig(series)} className={`h-[300px] w-full`}>
      <LineChart
        accessibilityLayer
        data={data}
        margin={{
          left: 12,
          right: 12,
        }}
      >
        <CartesianGrid vertical={false} />
        <XAxis dataKey="date" tickLine={false} axisLine={false} tickMargin={8} />
        <YAxis hide type="number" domain={[minAmount, maxAmount]} yAxisId="default" scale="log"
               padding={{ top: 20, bottom: 20 }} />
        <ChartTooltip cursor={false} content={<ChartTooltipContent />} />
        {series.map((it) => (
          <Line dataKey={it.name} type="monotone" stroke={it.color} strokeWidth={2} dot={false} activeDot
                yAxisId="default" />
        ))}
      </LineChart>
    </ChartContainer>
  );
}
