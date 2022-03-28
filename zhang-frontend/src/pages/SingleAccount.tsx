import AccountBalanceCheckLine from '@/components/AccountBalanceCheckLine';
import AccountDocumentLine from '@/components/AccountDocumentLine';
import Amount from '@/components/Amount';
import BalanceCheckLine from '@/components/BalanceCheckLine';
import BalancePadLine from '@/components/BalancePadLine';
import Block from '@/components/Block';
import TransactionLine from '@/components/TransactionLine';
import { gql, useQuery } from '@apollo/client';
import { Badge, Heading, Tab, Table, TableCaption, TabList, TabPanel, TabPanels, Tabs, Tbody, Td, Tfoot, Th, Thead, Tr } from '@chakra-ui/react'
import { useParams } from "react-router";

function SingleAccount() {

  let { accountName } = useParams();

  const { loading, error, data } = useQuery(gql`
    query SINGLE_ACCONT_JOURNAL($name: String) {
        account(name: $name) {
            name
            status
            currencies {
              name
            }
            snapshot {
              detail {
                number
                currency
              }
            }
            documents {
              filename
              __typename
            }
            journals {
                date
                type: __typename
                ... on TransactionDto {
                  payee
                  narration
                  postings {
                    account {
                      name
                    }
                    unit {
                      number
                      currency
                    }
                  }
                }
                ... on BalanceCheckDto {
                  account {
                    name
                  }
                  balanceAmount {
                    number
                    currency
                  }
                  currentAmount {
                    number
                    currency
                  }
                  isBalanced
                  distance {
                    number
                    currency
                  }
                }
              }
        }
      }    
`, {
    variables: {
      name: accountName
    }
  });
  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;


  return (
    <div>
      <Heading><Badge variant='outline' colorScheme={"green"}>{data.account.status}</Badge>{accountName}</Heading>

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
                loading ? <p>Loading...</p> :
                  error ? <p>Error :(</p> :
                    data.account.journals.map((journal) => {
                      switch (journal.type) {
                        case "BalanceCheckDto":
                          return <BalanceCheckLine data={journal} />
                          break;
                        case "BalancePadDto":
                          return <BalancePadLine data={journal} />
                          break;
                        case "TransactionDto":
                          return <TransactionLine data={journal} />
                          break;
                      }
                    })

              }


            </TabPanel>
            <TabPanel >
              {data.account.documents.map(document => {
                switch (document.__typename) {
                  case "AccountDocumentDto":
                    return (
                      <AccountDocumentLine filename={document.filename} account={{
                        name: data.account.name,
                        status: data.account.status
                      }} />
                    )
                  case "TransactionDocumentDto":
                    return (
                      <div>todo transaction document dto line</div>
                    )
                }
              })}

            </TabPanel>
            <TabPanel >
              <Block title='Balance Check'>

                <Table variant='simple'>
                  <Thead>
                    <Tr>
                      <Th>Currency</Th>
                      <Th>Current Balance</Th>
                      <Th isNumeric>Distanation</Th>
                    </Tr>
                  </Thead>
                  <Tbody>
                    {data.account.currencies.map(it => (
                      <Tr>
                        <Td>{it.name}</Td>
                        <Td><Amount amount={data.account.snapshot.detail.find(cur => cur.currency === it.name)?.number || "0.00"} currency={it.name} /></Td>
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
