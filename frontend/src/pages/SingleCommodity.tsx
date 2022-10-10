import { useQuery } from '@apollo/client';
import { Container, Title, Tabs, Table } from '@mantine/core';
import { format } from 'date-fns';
import { useParams } from 'react-router';
import Amount from '../components/Amount';
import { SingleCommodityQuery, SINGLE_COMMODITIY } from '../gql/singleCommodity';

export default function SingleCommodity() {
  let { commodityName } = useParams();

  const { loading, error, data } = useQuery<SingleCommodityQuery>(SINGLE_COMMODITIY, {
    variables: {
      name: commodityName,
    },
  });
  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;

  return (
    <Container fluid>
      <Title order={2}>{commodityName}</Title>

      <Tabs defaultValue="lots" mt="lg">
        <Tabs.List>
          <Tabs.Tab value="lots">Lots</Tabs.Tab>
          <Tabs.Tab value="price_history">Price History</Tabs.Tab>
        </Tabs.List>

        <Tabs.Panel value="lots" pt="xs">
          <Table verticalSpacing="xs" highlightOnHover>
            <thead>
              <tr>
                <th>Lot</th>
                <th>Balance</th>
              </tr>
            </thead>
            <tbody>
              {data?.currency.lots.map((it, idx) => (
                <tr key={idx}>
                  <td>
                    {it.lotPrice} {it.lotCurrency}
                  </td>
                  <td>{it.number}</td>
                </tr>
              ))}
            </tbody>
          </Table>
        </Tabs.Panel>

        <Tabs.Panel value="price_history" pt="xs">
          <Table verticalSpacing="xs" highlightOnHover>
            <thead>
              <tr>
                <th>Date</th>
                <th>Price</th>
              </tr>
            </thead>
            <tbody>
              {data?.currency.priceHistories.map((it, idx) => (
                <tr key={idx}>
                  <td>{format(new Date(it.date * 1000), 'yyyy-MM-dd')}</td>
                  <td>
                    <Amount amount={it.amount.number} currency={it.amount.currency} />
                  </td>
                </tr>
              ))}
            </tbody>
          </Table>
        </Tabs.Panel>
      </Tabs>
    </Container>
  );
}
