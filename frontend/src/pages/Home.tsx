import { useQuery } from "@apollo/client";
import { Box, Flex } from "@chakra-ui/react";
import { format } from "date-fns";
import React from 'react';
import { Chart } from 'react-chartjs-2';
import Block from "../components/Block";
import ErrorBox from "../components/ErrorBox";
import { STATISTIC, StatisticResponse } from "../gql/statistic";

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
    y: {
      type: 'linear' as const,
      display: true,
      position: 'left' as const,
      beginAtZero: false
    },
    y1: {
      type: 'linear' as const,
      display: true,
      position: 'right' as const,
      grid: {
        drawOnChartArea: false,
      },
      beginAtZero: false
    },
  },
};
const build_chart_data = (data: StatisticResponse) => {
  const labels = data.statistic.frames.map(frame => format(new Date(frame.end * 1000), 'MMM dd'));
  const total_dataset = data.statistic.frames.map(frame => parseFloat(frame.total.summary.number));
  const income_dataset = data.statistic.frames.map(frame => -1 * parseFloat(frame.income.summary.number));
  const expense_dataset = data.statistic.frames.map(frame => parseFloat(frame.expense.summary.number));
  return {
    labels,
    datasets: [
      {
        type: 'line' as const,
        label: 'total',
        borderColor: 'rgb(255, 99, 132)',
        borderWidth: 2,
        fill: false,
        data: total_dataset,
        yAxisID: 'y',
      },
      {
        type: 'bar' as const,
        label: 'income',
        backgroundColor: 'rgb(75, 192, 192)',
        data: income_dataset,
        borderColor: 'white',
        borderWidth: 2,
        yAxisID: 'y1',
      },
      {
        type: 'bar' as const,
        label: 'expense',
        backgroundColor: 'rgb(53, 162, 235)',
        data: expense_dataset,
        yAxisID: 'y1',
      },
    ],
  }
}

function Home() {
  const now = new Date();
  const begining_time = new Date(now.getFullYear(), now.getMonth(), 1, 0, 0, 1);
  const end_time = new Date(now.getFullYear(), now.getMonth() + 1, 0, 23, 59, 59);

  const { loading, error, data } = useQuery<StatisticResponse>(STATISTIC, {
    variables: {
      from: Math.round(begining_time.getTime() / 1000),
      to: Math.round(end_time.getTime() / 1000),
      gap: 1
    }
  });
  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;

  return (
    <Box as="section">
      <Flex>
        <Box flex={"0 0 70%"} m={1}>
          <Block title="statistic">
            <Chart type='bar' data={build_chart_data(data!)} options={options} />
          </Block>
        </Box>
        <Box flex={1} minW={0} m={1}>
          <ErrorBox></ErrorBox>
        </Box>
      </Flex>
    </Box>
  )
}

export default Home;
