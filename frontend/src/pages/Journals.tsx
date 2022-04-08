import { useQuery } from "@apollo/client";
import { Heading } from '@chakra-ui/react';
import JournalLine from "../components/JournalLine";
import { JouralListQuery, JOURNAL_LIST } from "../gql/jouralList";
function Journals() {
  const { loading, error, data } = useQuery<JouralListQuery>(JOURNAL_LIST);

  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;


  return (
    <div>
      <Heading mx={4} my={4}>{data?.journals.length} Journals</Heading>
      <div>
        {data?.journals.map((journal, idx) => <JournalLine key={idx} data={journal} />)
        }
      </div>
    </div>

  );
}

export default Journals;
