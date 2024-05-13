import { Box, Container, SimpleGrid, Title } from '@mantine/core';
import { LoadingState } from '../rest-model';
import { useAppSelector } from '../states';
import { Heading } from '../components/basic/Heading';
import { groupBy } from 'lodash-es';
import CommodityBox from '../components/CommodityBox';
import { useDocumentTitle } from '@mantine/hooks';

const FRONTEND_DEFAULT_GROUP = '__ZHANG__FRONTEND_DEFAULT__GROUP__';
export default function Commodities() {
  const { value: commodities, status } = useAppSelector((state) => state.commodities);
  const ledgerTitle = useAppSelector((state) => state.basic.title ?? 'Zhang Accounting');

  useDocumentTitle(`Commodities - ${ledgerTitle}`);

  if (status === LoadingState.Loading || status === LoadingState.NotReady) return <>loading</>;

  const groupedCommodities = groupBy(commodities, (it) => it.group ?? FRONTEND_DEFAULT_GROUP);

  return (
    <Container fluid>
      <Heading title={`Commodities`}></Heading>

      {FRONTEND_DEFAULT_GROUP in groupedCommodities && (
        <Box mt={'lg'}>
          <SimpleGrid
            cols={{ base: 4, md: 2, sm: 1 }}
            spacing={{ base: 'md', sm: 'sm' }}
          >
            {groupedCommodities[FRONTEND_DEFAULT_GROUP].map((commodity) => (
              <CommodityBox {...commodity} operating_currency={false}></CommodityBox>
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
            <SimpleGrid
              cols={{ base: 4, md: 2, sm: 1 }}
              spacing={{ base: 'md', sm: 'sm' }}
            >
              {groupedCommodities[groupName].map((commodity) => (
                <CommodityBox {...commodity} operating_currency={false}></CommodityBox>
              ))}
            </SimpleGrid>
          </Box>
        ))}
    </Container>
  );
}
