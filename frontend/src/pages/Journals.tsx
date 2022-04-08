import BalanceCheckLine from "../components/BalanceCheckLine";
import BalancePadLine from "../components/BalancePadLine";
import TransactionLine from "../components/TransactionLine";
import { useQuery } from "@apollo/client";
import { Heading } from '@chakra-ui/react';
import { JouralListQuery, JOURNAL_LIST } from "../gql/jouralList";
function Journals() {
  const { loading, error, data } = useQuery<JouralListQuery>(JOURNAL_LIST);

  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;


  return (
    <div>
      <Heading mx={4} my={4}>{data?.journals.length} Journals</Heading>
      <div>
        {data?.journals.map((journal) => {
          switch (journal.type) {
            case "BalanceCheckDto":
              return <BalanceCheckLine data={journal} />
            case "BalancePadDto":
              return <BalancePadLine data={journal} />
            case "TransactionDto":
              return <TransactionLine data={journal} />
            default:
              return (<div></div>)
          }
        })
        }
      </div>
    </div>

  );
}

export default Journals;
