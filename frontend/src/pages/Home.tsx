import { useQuery } from '@apollo/client';
import { Container, Title, Grid } from '@mantine/core';
import { format } from 'date-fns';
import { Chart } from 'react-chartjs-2';
import { useTranslation } from 'react-i18next';

import ErrorBox from '../components/ErrorBox';
import StatisticBar from '../components/StatisticBar';
import { STATISTIC, StatisticResponse } from '../gql/statistic';

const options = {
  responsive: true,
  interaction: {
    mode: 'index' as const,
    intersect: false,
  },
  stacked: false,
  plugins: {
    title: {
      display: true,
      text: 'Current Month Statistic',
    },
  },
  scales: {
    total: {
      type: 'linear' as const,
      display: true,
      position: 'left' as const,
      beginAtZero: false,
    },
    bar: {
      type: 'logarithmic' as const,
      display: true,
      position: 'right' as const,
      grid: {
        drawOnChartArea: false,
      },
      beginAtZero: false,
    },
  },
};
const build_chart_data = (data: StatisticResponse) => {
  const labels = data.statistic.frames.map((frame) => format(new Date(frame.end * 1000), 'MMM dd'));
  const total_dataset = data.statistic.frames.map((frame) => -1 * parseFloat(frame.total.summary.number));
  console.log('total_dataset', total_dataset);
  const income_dataset = data.statistic.frames.map((frame) => -1 * parseFloat(frame.income.summary.number));
  const expense_dataset = data.statistic.frames.map((frame) => parseFloat(frame.expense.summary.number));
  console.log('income_dataset', income_dataset, expense_dataset);
  return {
    labels,
    datasets: [
      {
        type: 'line' as const,
        label: 'total',
        borderColor: 'rgb(255, 99, 132)',
        borderWidth: 2,
        data: total_dataset,
        yAxisID: 'total',
      },
      {
        type: 'bar' as const,
        label: 'income',
        backgroundColor: 'rgb(17, 183, 205)',
        data: income_dataset,
        borderColor: 'white',
        borderRadius: 3,
        yAxisID: 'bar',
      },
      {
        type: 'bar' as const,
        label: 'expense',
        backgroundColor: 'rgb(247, 31, 167)',
        borderRadius: 3,
        data: expense_dataset,
        yAxisID: 'bar',
      },
    ],
  };
};

function Home() {
  const { t } = useTranslation();
  const now = new Date();
  const begining_time = new Date(now.getFullYear(), now.getMonth(), 1, 0, 0, 1);
  const end_time = new Date(now.getFullYear(), now.getMonth() + 1, 0, 23, 59, 59);

  const { loading, error, data } = useQuery<StatisticResponse>(STATISTIC, {
    variables: {
      from: Math.round(begining_time.getTime() / 1000),
      to: Math.round(end_time.getTime() / 1000),
      gap: 1,
    },
  });
  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;

  return (
    <Container fluid>
      <Title order={2}>{t('Dashboard')}</Title>
      <StatisticBar />
      <Grid>
        <Grid.Col span={8}>
          <Chart type="line" data={build_chart_data(data!)} options={options} />
        </Grid.Col>
        <Grid.Col span={4}>
          <ErrorBox></ErrorBox>
        </Grid.Col>
      </Grid>
    </Container>
  );
}

export default Home;
