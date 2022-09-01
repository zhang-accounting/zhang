import { useQuery } from "@apollo/client";
import { Box, Button, CloseButton, Flex, Heading } from '@chakra-ui/react';
import { useState } from "react";
import JournalLine from "../components/JournalLine";
import JournalPreview from "../components/journalPreview/JournalPreview";
import { JouralListQuery, JournalItem, JOURNAL_LIST } from "../gql/jouralList";
function Journals() {
  const [page, setPage] = useState(1);
  const { loading, error, data, fetchMore } = useQuery<JouralListQuery>(JOURNAL_LIST, { variables: { page: 1 } });
  const [selectedJournal, setSelectedJournal] = useState<JournalItem | null>(null)
  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;

  const onMoreButtonClick = () => {
    fetchMore({
      variables: {
        page: page + 1
      }
    });
    setPage(page + 1);
  }
  return (
    <Flex>
      <Box flex="0 0 60%" h="calc(100vh - var(--chakra-sizes-20))" maxH="calc(100vh - var(--chakra-sizes-20))" overflow="scroll" borderRight="1px">
        <Heading mx={4} my={4}>{data?.journals.data.length} Journals</Heading>
        <div>
          {data?.journals.data.map((journal, idx) => <JournalLine key={idx} data={journal} setSelectedJournal={setSelectedJournal} />)}
        </div>
        <Button onClick={onMoreButtonClick}>Fetch More</Button>
      </Box>
      {selectedJournal ?
        <Box flex=" 0 0 40%">
          <div><CloseButton onClick={() => setSelectedJournal(null)} /></div>
          <JournalPreview data={selectedJournal!} />

        </Box> :
        <Box>
          select one directive to show detail
        </Box>
      }
    </Flex>
  );
}

export default Journals;
