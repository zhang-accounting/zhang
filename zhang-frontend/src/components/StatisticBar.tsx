import { gql, useQuery } from "@apollo/client";
import { Flex, Text, Tooltip } from "@chakra-ui/react";
import Amount from "./Amount";
import StatisticBox from "./StatisticBox";

export default function Component({ }) {
    const { loading, error, data } = useQuery(gql`
    query BAR_STATISTIC {
        statistic {
          total {
            summary {
              number
              currency
            }
            detail {
              number
              currency
            }
          }
          income {
            summary {
              number
              currency
            }
            detail {
              number
              currency
            }
          }
          expense {
            summary {
              number
              currency
            }
            detail {
              number
              currency
            }
          }
          liability {
            summary {
              number
              currency
            }
            detail {
              number
              currency
            }
          }
        }
      }      
    `)
    if (loading) return <p>Loading...</p>;
    if (error) return <p>Error :(</p>;  
    return (
        <Flex >
            <StatisticBox text={"资产余额"} amount={data.statistic.total.summary.number} currency={data.statistic.total.summary.currency} />
            <StatisticBox text={"本月收入"} amount={data.statistic.income.summary.number} currency={data.statistic.income.summary.currency} negetive />
            <StatisticBox text={"本月支出"} amount={data.statistic.expense.summary.number} currency={data.statistic.expense.summary.currency} />
            <StatisticBox text={"负债"} amount={data.statistic.liability.summary.number} currency={data.statistic.liability.summary.currency} negetive />
        </Flex>
    )
}