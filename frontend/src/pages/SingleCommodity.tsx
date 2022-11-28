import { Container, Table, Tabs, Title } from '@mantine/core';
import { format } from 'date-fns';
import { useParams } from 'react-router';
import useSWR from 'swr';
import { fetcher } from '..';
import Amount from '../components/Amount';
import { CommodityDetail } from '../rest-model';

export default function SingleCommodity() {
  let { commodityName } = useParams();
  const {data, error} = useSWR<CommodityDetail>(`/api/commodities/${commodityName}`, fetcher);
  
  if (error) return <div>failed to load</div>;
  if (!data) return <div>loading123</div>;

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
                <th>Account</th>
                <th>Lot</th>
                <th>Balance</th>
              </tr>
            </thead>
            <tbody>
              {data.lots.map((it, idx) => (
                <tr key={idx}>
                  <td>{it.account}</td>
                  <td>
                    {it.price_amount} {it.price_commodity}
                  </td>
                  <td>{it.amount}</td>
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
              {data.prices.map((it, idx) => (
                <tr key={idx}>
                  <td>{format(new Date(it.datetime), 'yyyy-MM-dd')}</td>
                  <td>
                    <Amount amount={it.amount} currency={it.target_commodity} />
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
