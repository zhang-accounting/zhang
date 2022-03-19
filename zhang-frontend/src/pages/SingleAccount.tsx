import BalanceCheckLine from '@/components/BalanceCheckLine';
import BalancePadLine from '@/components/BalancePadLine';
import TransactionLine from '@/components/TransactionLine';
import { gql, useQuery } from '@apollo/client';
import { Badge, Heading, Tab, TabList, TabPanel, TabPanels, Tabs } from '@chakra-ui/react'
import { useParams } from "react-router";

function SingleAccount() {

    let { accountName } = useParams();

    const { loading, error, data } = useQuery(gql`
    query SINGLE_ACCONT_JOURNAL($name: String) {
        account(name: $name) {
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

    return (
        <div>
            <Heading><Badge variant='outline' colorScheme={"green"}>OPEN</Badge>{accountName}</Heading>

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
                        <TabPanel >Documents</TabPanel>
                        <TabPanel >Settings</TabPanel>
                    </TabPanels>
                </Tabs>
            </div>
        </div>

    );
}

export default SingleAccount;
