import { gql, useQuery } from "@apollo/client";
import { Box, Container, SimpleGrid, Stat } from "@chakra-ui/react";

function Home() {
 
  const stats = [
    { label: 'Total Subscribers', value: '71,887' },
    { label: 'Avg. Open Rate', value: '56.87%' },
    { label: 'Avg. Click Rate', value: '12.87%' },
  ]
 
  return (
    <Box as="section" py={{ base: '4', md: '8' }}>
    <Container>
      hello home
      <SimpleGrid columns={{ base: 1, md: 3 }} gap={{ base: '5', md: '6' }}>
        {stats.map(({ label, value }) => (
          <Stat key={label} label={label} value={value} />
        ))}
      </SimpleGrid>
    </Container>
  </Box>
  )
}

export default Home;
