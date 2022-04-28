import { useQuery } from "@apollo/client";
import { Box, CloseButton, Flex, Heading } from '@chakra-ui/react';
import { useState } from "react";
import Block from "../components/Block";
import JournalLine from "../components/JournalLine";
import JournalPreview from "../components/journalPreview/JournalPreview";
import { JouralListQuery, JournalItem, JOURNAL_LIST } from "../gql/jouralList";
function Journals() {
  const { loading, error, data } = useQuery<JouralListQuery>(JOURNAL_LIST);
  const [selectedJournal, setSelectedJournal] = useState<JournalItem | null>(null)
  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;


  return (
    <Flex>
      <Box flex="1">
        <Heading mx={4} my={4}>{data?.journals.length} Journals</Heading>
        <div>
          {data?.journals.map((journal, idx) => <JournalLine key={idx} data={journal} setSelectedJournal={setSelectedJournal} />)
          }
        </div>
      </Box>
      {selectedJournal &&
        <Box flex=" 0 0 30%">
          <div><CloseButton onClick={() => setSelectedJournal(null)} /></div>
          <JournalPreview data={selectedJournal!} />

        </Box>
      }

    </Flex>


  );
}

export default Journals;
