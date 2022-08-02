import { useQuery } from '@apollo/client';
import { Heading, Tab, Table, TabList, TabPanel, TabPanels, Tabs, Tbody, Td, Th, Thead, Tr } from '@chakra-ui/react';
import { format } from 'date-fns';
import { useParams } from "react-router";
import Amount from '../components/Amount';
import { SingleCommodityQuery, SINGLE_COMMODITIY } from '../gql/singleCommodity';

export default function SingleCommodity() {

  let { commodityName } = useParams();

  const { loading, error, data } = useQuery<SingleCommodityQuery>(SINGLE_COMMODITIY, {
    variables: {
      name: commodityName
    }
  });
  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;


  return (
    <div>
      <Heading>{commodityName}</Heading>

      <div>
        <Tabs isLazy>
          <TabList>
            <Tab>Lots</Tab>
            <Tab>Price History</Tab>
          </TabList>

          <TabPanels>
            <TabPanel >
              <Table variant='simple'>
                <Thead>
                  <Tr>
                    <Th>Lot</Th>
                    <Th isNumeric>Balance</Th>
                  </Tr>
                </Thead>
                <Tbody>
                  {data?.currency.lots.map((it, idx) => (
                    <Tr key={idx}>
                      <Td>{it.lotPrice} {it.lotCurrency}</Td>
                      <Td isNumeric>{it.number}</Td>
                    </Tr>

                  ))}
                </Tbody>
              </Table>
            </TabPanel>
            <TabPanel >
              <Table variant='simple'>
                <Thead>
                  <Tr>
                    <Th>Date</Th>
                    <Th isNumeric>Price</Th>
                  </Tr>
                </Thead>
                <Tbody>
                  {data?.currency.priceHistories.map((it, idx) => (
                    <Tr key={idx}>
                      <Td>{format(new Date(it.date * 1000), "yyyy-MM-dd")}</Td>
                      <Td isNumeric><Amount amount={it.amount.number} currency={it.amount.currency} /></Td>
                    </Tr>

                  ))}
                </Tbody>
              </Table>
            </TabPanel>

          </TabPanels>
        </Tabs>
      </div>
    </div>

  );
}
