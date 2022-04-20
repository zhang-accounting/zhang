import { Box } from "@chakra-ui/react";

import React from 'react';
import {
  Chart as ChartJS,
  LinearScale,
  CategoryScale,
  BarElement,
  PointElement,
  LineElement,
  Legend,
  Tooltip,
} from 'chart.js';
import { Chart } from 'react-chartjs-2';
import Block from "../components/Block";


const labels = ['January', 'February', 'March', 'April', 'May', 'June', 'July'];

export const data = {
  labels,
  datasets: [
    {
      type: 'line' as const,
      label: 'Dataset 1',
      borderColor: 'rgb(255, 99, 132)',
      borderWidth: 2,
      fill: false,
      data: [1000, 2000, 1000, 1000, 2000, 3000, 3000],
      yAxisID: 'y',
    },
    {
      type: 'bar' as const,
      label: 'Dataset 2',
      backgroundColor: 'rgb(75, 192, 192)',
      data: [1, 1, 1, 1, 1, 1, 1],
      borderColor: 'white',
      borderWidth: 2,
      yAxisID: 'y1',
    },
    {
      type: 'bar' as const,
      label: 'Dataset 3',
      backgroundColor: 'rgb(53, 162, 235)',
      data: [10, 10, 10, 10, 10, 10, 10],
      yAxisID: 'y1',
    },
  ],
};

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
      text: 'Chart.js Line Chart - Multi Axis',
    },
  },
  scales: {
    y: {
      type: 'linear' as const,
      display: true,
      position: 'left' as const,
    },
    y1: {
      type: 'linear' as const,
      display: true,
      position: 'right' as const,
      grid: {
        drawOnChartArea: false,
      },
    },
  },
};

function Home() {



  return (
    <Box as="section">
      <Box>
        <Block title="statistic">
          <Chart type='bar' data={data} options={options} />
        </Block>
      </Box>
    </Box>
  )
}

export default Home;
