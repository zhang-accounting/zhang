import { useQuery } from '@apollo/client';
import { Badge, Heading, Tab, Table, TabList, TabPanel, TabPanels, Tabs, Tbody, Td, Th, Thead, Tr } from '@chakra-ui/react';
import { useParams } from "react-router";
import AccountBalanceCheckLine from '../components/AccountBalanceCheckLine';
import AccountDocumentLine from '../components/documentLines/AccountDocumentLine';
import AccountDocumentUpload from '../components/AccountDocumentUpload';
import Amount from '../components/Amount';
import Block from '../components/Block';
import JournalLine from '../components/JournalLine';
import { AccountItem, CommodityBalanceTime } from '../gql/accountList';
import { SingleAccountJournalQuery, SINGLE_ACCONT_JOURNAL } from '../gql/singleAccount';
import { maxBy } from 'lodash'
import { format } from 'date-fns';
function SingleAccount() {

  let { accountName } = useParams();

  const getLatestBalanceTime = (commodity: string, times: CommodityBalanceTime[]) => {
    const latestTime = maxBy(times.filter(time => time.commodity === commodity), time => time.date)
    if (latestTime) {
      return format(new Date(latestTime.date * 1000), "yyyy-MM-dd");
    } else {
      return 'N/A'
    }
  }

  const { loading, error, data } = useQuery<SingleAccountJournalQuery>(SINGLE_ACCONT_JOURNAL, {
    variables: {
      name: accountName
    }
  });
  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;



  return (
    <div>
      <Heading><Badge variant='outline' colorScheme={"green"}>{data?.account.status}</Badge>{accountName}</Heading>

      <div>
        <Tabs isLazy>
          <TabList>
            <Tab>Journals</Tab>
            <Tab>Documents</Tab>
            <Tab>Settings</Tab>
          </TabList>

          <TabPanels>
            <TabPanel >
              {
                data?.account.journals.map((journal, idx) => <JournalLine key={idx} data={journal} />)
              }
            </TabPanel>
            <TabPanel >
              <Block title='Document Upload'>
                <AccountDocumentUpload accountName={data!.account.name} />
              </Block>
              <Block title='Documents'>
                <div>{data?.account.documents.map((document, idx) => {
                  switch (document.__typename) {
                    case "AccountDocumentDto":
                      return (
                        <AccountDocumentLine key={idx} filename={document.filename} account={{
                          name: data.account.name,
                          status: data.account.status
                        } as AccountItem} />
                      )
                    case "TransactionDocumentDto":
                      return (
                        <div key={idx}>todo transaction document dto line</div>
                      )
                    default:
                      return <div></div>
                  }
                })}</div>
              </Block>

            </TabPanel>
            <TabPanel >
              <Block title='Balance Check'>

                <Table variant='simple'>
                  <Thead>
                    <Tr>
                      <Th>Currency</Th>
                      <Th>Current Balance</Th>
                      <Th>Latest Balance Time</Th>
                      <Th isNumeric>Distanation</Th>
                    </Tr>
                  </Thead>
                  <Tbody>
                    {data?.account.currencies.map((it, idx) => (
                      <Tr key={idx}>
                        <Td>{it.name}</Td>
                        <Td><Amount amount={data.account.snapshot.detail.find(cur => cur.currency === it.name)?.number || "0.00"} currency={it.name} /></Td>
                        <Td>{getLatestBalanceTime(it.name, data!.account.latestBalanceTimes)}</Td>
                        <Td isNumeric><AccountBalanceCheckLine currency={it.name} accountName={data.account.name} /></Td>
                      </Tr>
                    ))}
                  </Tbody>
                </Table>

              </Block>

            </TabPanel>
          </TabPanels>
        </Tabs>
      </div>
    </div>

  );
}

export default SingleAccount;
