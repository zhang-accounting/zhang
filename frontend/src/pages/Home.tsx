import { Container, Grid, Title } from '@mantine/core';
import { format } from 'date-fns';
import { Chart } from 'react-chartjs-2';
import { useTranslation } from 'react-i18next';

import BigNumber from 'bignumber.js';
import { sortBy } from 'lodash-es';
import useSWR from "swr";
import ErrorBox from '../components/ErrorBox';
import Section from '../components/Section';
import StatisticBar from '../components/StatisticBar';
import { fetcher } from "../index";
import { AccountType, StatisticResponse } from '../rest-model';
import { useAppSelector } from '../states';

const options = (meta: { isLogarithmic: boolean, offset: number, max: number }) => ({
  responsive: true,
  interaction: {
    mode: 'index' as const,
    intersect: false,
  },
  stacked: false,
  plugins: {
    tooltip: {
      position: 'nearest' as const,
      callbacks: {

        title: (item: any) => {
          return item[0].label
        },
        label: (item: any) => {

          if (item.dataset.label === 'total') {
            const valueWithOffset = parseFloat(item.formattedValue) + meta.offset;
            return `${item.dataset.label}: ${valueWithOffset} CNY`
          }
          return `${item.dataset.label}: ${item.formattedValue} CNY`
        }
      }
    }
  },
  scales: {
    x: {
      display: true,
      grid: {
        display: false
      }
    },
    total: {
      type: meta.isLogarithmic ? 'logarithmic' as const : 'linear' as const,
      display: false,
      position: 'left' as const,
      beginAtZero: false,
      suggestedMax: meta.max,
      ticks: {
        callback: function (value: any, _index: any, _ticks: any) {
          return parseFloat(value) + meta.offset;
        }
      }

    },
    bar: {
      type: 'linear' as const,
      display: false,
      position: 'right' as const,
      grid: {
        drawOnChartArea: false,
      },
    },
  },
});
const build_chart_data = (data: StatisticResponse) => {
  const dates = sortBy(Object.keys(data.changes).map(date => [date, new Date(date)]), item => item[1]);

  const sequencedDate = dates.map(date => date[0] as string);

  const labels = dates.map((date) => format(date[1] as Date, 'MMM dd'));

  let total_dataset = sequencedDate.map(date => {
    const target_day = data.details[date] ?? {};
    let total = new BigNumber(0);
    Object.entries(target_day).filter(it =>
      it[0].startsWith(AccountType.Assets) || it[0].startsWith(AccountType.Liabilities)
    ).forEach(it => {
      total = total.plus(new BigNumber(it[1].number))
    })
    return total.toNumber();
  });

  // let total_dataset = data.statistic.frames.map((frame) => parseFloat(frame.total.summary.number));
  const isLogarithmic = total_dataset.every(item => item >= 0);
  let min = 0;
  let max = Math.max.apply(0, total_dataset) + 50;

  if (isLogarithmic) {
    min = Math.min.apply(0, total_dataset) - 50;
    max = max - min;
    total_dataset = total_dataset.map(item => item - min);
  }

  const income_dataset = sequencedDate.map(date => -1 * parseFloat(data.changes[date]?.[AccountType.Income]?.number ?? 0))
  const expense_dataset = sequencedDate.map(date => parseFloat(data.changes[date]?.[AccountType.Expenses]?.number ?? 0))
  return {
    data: {
      labels,
      datasets: [
        {
          type: 'line' as const,
          label: 'total',
          borderColor: '#2E94B9',
          borderWidth: 2,
          data: total_dataset,
          pointRadius: 0,
          hoverBackgroundColor: "#2E94B9",
          yAxisID: 'total',
        },
        {
          type: 'bar' as const,
          label: 'income',
          backgroundColor: 'rgba(46,148,185,0.5)',
          hoverBackgroundColor: "rgba(46,148,185,0.85)",
          data: income_dataset,
          borderColor: 'white',
          borderRadius: 2,
          yAxisID: 'bar',
        },
        {
          type: 'bar' as const,
          label: 'expense',
          backgroundColor: 'rgba(210,85,101,0.5)',
          hoverBackgroundColor: "rgba(210,85,101,0.85)",
          borderRadius: 2,
          data: expense_dataset,
          yAxisID: 'bar',
        },
      ],
    }, meta: { isLogarithmic, offset: min, max }
  };
};

function Home() {
  const { t } = useTranslation();
  const error_total_number = useAppSelector(state => state.errors.total_number)
  const now = new Date();
  const beginning_time = new Date(now.getFullYear(), now.getMonth() - 1, now.getDate(), 0, 0, 1);
  const end_time = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 23, 59, 59);

  const { data, error } = useSWR<StatisticResponse>(`/api/statistic?from=${beginning_time.toISOString()}&to=${end_time.toISOString()}&interval=Day`, fetcher)

 

  if (error) return <div>failed to load</div>;
  if (!data) return <>loading</>;
  const chart_info = build_chart_data(data);
  return (
    <Container fluid >
      <Title order={2}>{t('Dashboard')}</Title>
      <StatisticBar />
      <Grid>
        <Grid.Col span={8}>
          <Section title="Current Statistics">
            <Chart type="line" data={chart_info.data} options={options(chart_info.meta)} />
          </Section>
        </Grid.Col>
        <Grid.Col span={4}>
          <Section title={`${error_total_number} Errors`}>
            <ErrorBox></ErrorBox>
          </Section>
        </Grid.Col>
      </Grid>
    </Container>
  );
}

export default Home;
