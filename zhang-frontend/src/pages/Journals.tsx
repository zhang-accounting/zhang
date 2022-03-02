import BalanceCheckLine from "@/components/BalanceCheckLine";
import BalancePadLine from "@/components/BalancePadLine";
import TransactionLine from "@/components/TransactionLine";
import { gql, useQuery } from "@apollo/client";
import { Heading } from '@chakra-ui/react'
src/utils/account-trie
function Journals() {
  const { loading, error, data } = useQuery(gql`
  query {
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
  
`);

  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;


  return (
    <div>
      <Heading mx={4} my={4}>{data.journals.length} Journals</Heading>
      <div>
      {data.journals.map((journal) => {
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
    </div>
    </div>

  );
}

export default Journals;
