import Block from "@/components/Block";
import { gql, useQuery } from "@apollo/client";
import { Box, Container, SimpleGrid, Stat } from "@chakra-ui/react";

function Home() {
 
  const stats = [
    { label: 'Total Subscribers', value: '71,887' },
    { label: 'Avg. Open Rate', value: '56.87%' },
    { label: 'Avg. Click Rate', value: '12.87%' },
  ]
 
  return (
    <Box as="section">
      <Box>
        {/* <Block title="Errors" />
        <Block title="Errors" />
        <Block title="Errors" />
        <Block title="Errors" />
        <Block title="Errors" /> */}
      </Box>
  </Box>
  )
}

export default Home;
