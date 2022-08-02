import { useQuery } from '@apollo/client';
import { Badge, Heading, Tab, Table, TabList, TabPanel, TabPanels, Tabs, Tbody, Td, Th, Thead, Tr } from '@chakra-ui/react';
import { useParams } from "react-router";
import AccountBalanceCheckLine from '../components/AccountBalanceCheckLine';
import AccountDocumentLine from '../components/documentLines/AccountDocumentLine';
import AccountDocumentUpload from '../components/AccountDocumentUpload';
import Amount from '../components/Amount';
import Block from '../components/Block';
import JournalLine from '../components/JournalLine';
import { AccountItem } from '../gql/accountList';
import { SingleAccountJournalQuery, SINGLE_ACCONT_JOURNAL } from '../gql/singleAccount';
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
            
          </TabPanels>
        </Tabs>
      </div>
    </div>

  );
}
