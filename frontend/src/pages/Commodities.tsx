import { Box, Container, SimpleGrid, Title } from '@mantine/core';
import { LoadingState } from '../rest-model';
import { useAppSelector } from '../states';
import { Heading } from '../components/basic/Heading';
import { groupBy } from 'lodash-es';
import CommodityBox from '../components/CommodityBox';

const FRONTEND_DEFAULT_GROUP = '__ZHANG__FRONTEND_DEFAULT__GROUP__';
export default function Commodities() {
  const { value: commodities, status } = useAppSelector((state) => state.commodities);

  if (status === LoadingState.Loading || status === LoadingState.NotReady) return <>loading</>;

  const groupedCommodities = groupBy(commodities, (it) => it.group ?? FRONTEND_DEFAULT_GROUP);
  console.log(groupedCommodities);

  return (
    <Container fluid>
      <Heading title={`Commodities`}></Heading>

      {FRONTEND_DEFAULT_GROUP in groupedCommodities && (
        <Box mt={'lg'}>
          <SimpleGrid
            cols={4}
            breakpoints={[
              { maxWidth: 'md', cols: 2, spacing: 'md' },
              { maxWidth: 'sm', cols: 1, spacing: 'sm' },
            ]}
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
            <Title fw={500} order={5} color={'dimmed'}>
              {groupName}
            </Title>
            <SimpleGrid
              cols={4}
              breakpoints={[
                { maxWidth: 'md', cols: 2, spacing: 'md' },
                { maxWidth: 'sm', cols: 1, spacing: 'sm' },
              ]}
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
