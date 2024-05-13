import { Container, Table, Tabs } from '@mantine/core';
import { format } from 'date-fns';
import { useParams } from 'react-router';
import useSWR from 'swr';
import { fetcher } from '..';
import Amount from '../components/Amount';
import { CommodityDetail } from '../rest-model';
import { Heading } from '../components/basic/Heading';
import { useAppSelector } from '../states';
import { useDocumentTitle } from '@mantine/hooks';

export default function SingleCommodity() {
  let { commodityName } = useParams();
  const { data, error } = useSWR<CommodityDetail>(`/api/commodities/${commodityName}`, fetcher);

  const ledgerTitle = useAppSelector((state) => state.basic.title ?? 'Zhang Accounting');

  useDocumentTitle(`${commodityName} | Commodities - ${ledgerTitle}`);

  if (error) return <div>failed to load</div>;
  if (!data) return <div>loading123</div>;

  return (
    <Container fluid>
      <Heading title={commodityName!}></Heading>
      <Tabs keepMounted={false} variant="outline" defaultValue="lots" mt="lg">
        <Tabs.List>
          <Tabs.Tab value="lots">Lots</Tabs.Tab>
          <Tabs.Tab value="price_history">Price History</Tabs.Tab>
        </Tabs.List>

        <Tabs.Panel value="lots" pt="xs">
          <Table verticalSpacing="xs" highlightOnHover>
            <Table.Thead>
              <Table.Tr>
                <Table.Th>Account</Table.Th>
                <Table.Th>Lot</Table.Th>
                <Table.Th>Balance</Table.Th>
              </Table.Tr>
            </Table.Thead>
            <tbody>
              {data.lots.map((it, idx) => (
                <Table.Tr key={idx}>
                  <Table.Td>{it.account}</Table.Td>
                  <Table.Td>
                    {it.price_amount} {it.price_commodity}
                  </Table.Td>
                  <Table.Td>{it.amount}</Table.Td>
                </Table.Tr>
              ))}
            </tbody>
          </Table>
        </Tabs.Panel>

        <Tabs.Panel value="price_history" pt="xs">
          <Table verticalSpacing="xs" highlightOnHover>
            <Table.Thead>
              <Table.Tr>
                <Table.Th>Date</Table.Th>
                <Table.Th>Price</Table.Th>
              </Table.Tr>
            </Table.Thead>
            <tbody>
              {data.prices.map((it, idx) => (
                <Table.Tr key={idx}>
                  <Table.Td>{format(new Date(it.datetime), 'yyyy-MM-dd')}</Table.Td>
                  <Table.Td>
                    <Amount amount={it.amount} currency={it.target_commodity} />
                  </Table.Td>
                </Table.Tr>
              ))}
            </tbody>
          </Table>
        </Tabs.Panel>
      </Tabs>
    </Container>
  );
}
