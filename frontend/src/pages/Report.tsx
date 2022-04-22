import { Box, Flex, Heading, Select, Progress, Stat, StatLabel, StatNumber } from "@chakra-ui/react";
// @ts-ignore
import DateRangePicker from '@wojtekmaj/react-daterange-picker';
import Block from "../components/Block";
import { Chart } from 'react-chartjs-2';
import { STATISTIC, StatisticResponse } from "../gql/statistic";
import { useQuery } from "@apollo/client";
import { format } from "date-fns";
import Amount from "../components/Amount";


const options = {
    responsive: true,
    interaction: {
        mode: 'index' as const,
        intersect: false,
    },
    stacked: false,
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
export default function Report() {
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

        <div>

            <Flex>
                <Box><Heading>Report</Heading></Box>
                <Box>
                    <DateRangePicker />
                </Box>

                <Box><Select>
                    <option selected value='option1'>Daily</option>
                    <option value='option2'>Weekly</option>
                    <option value='option3'>Monthly</option>
                </Select></Box>
            </Flex>
            <Flex>
                <Stat>
                    <StatLabel>资产余额</StatLabel>
                    <StatNumber><Amount amount={data?.statistic.total.summary.number} currency={data?.statistic.total.summary.currency} /></StatNumber>
                </Stat>
                <Stat>
                    <StatLabel>总收支</StatLabel>
                    <StatNumber><Amount amount={data?.statistic.incomeExpense.summary.number} currency={data?.statistic.incomeExpense.summary.currency} negetive /></StatNumber>
                </Stat>
                <Stat>
                    <StatLabel>收入</StatLabel>
                    <StatNumber><Amount amount={data?.statistic.income.summary.number} currency={data?.statistic.income.summary.currency} negetive /></StatNumber>
                </Stat>
                <Stat>
                    <StatLabel>支出</StatLabel>
                    <StatNumber><Amount amount={data?.statistic.expense.summary.number} currency={data?.statistic.expense.summary.currency} /></StatNumber>
                </Stat>
                <Stat>
                    <StatLabel>交易数</StatLabel>
                    <StatNumber>{data?.statistic.journals.length}</StatNumber>
                </Stat>
            </Flex>
            <Box m={1}>
                <Block title="Graph">
                    <Chart type='bar' data={build_chart_data(data!)} options={options} height={100} />
                </Block>
            </Box>

            <Flex>
                <Box flex="0 0 30%" m={1}>
                    <Block title="收入占比">
                        <Box pb={1}>
                            <p>AAA</p>
                            <Progress value={80} size='xs' />
                        </Box>
                        <Box >
                            <p>AAA</p>
                            <Progress value={80} size='xs' />
                        </Box>
                        <Box >
                            <p>AAA</p>
                            <Progress value={70} size='xs' />
                        </Box>
                        <Box >
                            <p>AAA</p>
                            <Progress value={15} size='xs' />
                        </Box>
                        <Box >
                            <p>AAA</p>
                            <Progress value={3} size='xs' />
                        </Box>
                        <Box >
                            <p>AAA</p>
                            <Progress value={2} size='xs' />
                        </Box>
                    </Block>
                </Box>
                <Box flex="1" m={1}>
                    <Block title="收入排行">
                        <p>Lines</p>
                    </Block>
                </Box>
            </Flex>
            <Flex>
                <Box flex="0 0 30%" m={1}>
                    <Block title="支出占比">
                        <Box pb={1}>
                            <p>AAA</p>
                            <Progress value={80} size='xs' />
                        </Box>
                        <Box >
                            <p>AAA</p>
                            <Progress value={80} size='xs' />
                        </Box>
                        <Box >
                            <p>AAA</p>
                            <Progress value={70} size='xs' />
                        </Box>
                        <Box >
                            <p>AAA</p>
                            <Progress value={15} size='xs' />
                        </Box>
                        <Box >
                            <p>AAA</p>
                            <Progress value={3} size='xs' />
                        </Box>
                        <Box >
                            <p>AAA</p>
                            <Progress value={2} size='xs' />
                        </Box>
                    </Block>
                </Box>
                <Box flex="1" m={1}>
                    <Block title="支出排行">
                        <p>Lines</p>
                    </Block>
                </Box>
            </Flex>




        </div>
    )
}


