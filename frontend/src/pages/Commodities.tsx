import { Box, Container, SimpleGrid, Title } from '@mantine/core';
import { Heading } from '../components/basic/Heading';
import CommodityBox from '../components/CommodityBox';
import { useDocumentTitle } from '@mantine/hooks';
import { useAtomValue } from 'jotai/index';
import { titleAtom } from '../states/basic';
import { FRONTEND_DEFAULT_GROUP, groupedCommoditiesAtom } from '../states/commodity';

export default function Commodities() {
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`Commodities - ${ledgerTitle}`);

  const groupedCommodities = useAtomValue(groupedCommoditiesAtom);

  return (
    <Container fluid>
      <Heading title={`Commodities`}></Heading>

      {FRONTEND_DEFAULT_GROUP in groupedCommodities && (
        <Box mt={'lg'}>
          <SimpleGrid cols={{ base: 1, md: 2, lg: 4 }} spacing={{ base: 'sm', md: 'md' }}>
            {groupedCommodities[FRONTEND_DEFAULT_GROUP].map((commodity) => (
              <CommodityBox key={commodity.name} {...commodity} operating_currency={false}></CommodityBox>
            ))}
          </SimpleGrid>
        </Box>
      )}
      {Object.keys(groupedCommodities)
        .filter((it) => it !== FRONTEND_DEFAULT_GROUP)
        .sort()
        .map((groupName) => (
          <Box mt={'lg'}>
            <Title fw={500} order={5} c={'dimmed'}>
              {groupName}
            </Title>
            <SimpleGrid cols={{ base: 1, md: 2, lg: 4 }} spacing={{ base: 'sm', md: 'md' }}>
              {groupedCommodities[groupName].map((commodity) => (
                <CommodityBox {...commodity} operating_currency={false}></CommodityBox>
              ))}
            </SimpleGrid>
          </Box>
        ))}
    </Container>
  );
}
