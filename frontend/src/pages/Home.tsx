import { Container, Grid } from '@mantine/core';

import useSWR from 'swr';
import ErrorBox from '../components/ErrorBox';
import Section from '../components/Section';
import StatisticBar from '../components/StatisticBar';
import { StatisticGraphResponse } from '../rest-model';
import ReportGraph from '../components/ReportGraph';
import { Heading } from '../components/basic/Heading';
import { useDocumentTitle } from '@mantine/hooks';
import { useAtomValue } from 'jotai';
import { errorCountAtom } from '../states/errors';
import { titleAtom } from '../states/basic';
import { fetcher } from '../global.ts';

function Home() {
  const error_total_number = useAtomValue(errorCountAtom);
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`Dashboard - ${ledgerTitle}`);

  const now = new Date();
  const beginning_time = new Date(now.getFullYear(), now.getMonth() - 1, now.getDate(), 0, 0, 1);
  const end_time = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 23, 59, 59);

  const { data, error } = useSWR<StatisticGraphResponse>(
    `/api/statistic/graph?from=${beginning_time.toISOString()}&to=${end_time.toISOString()}&interval=Day`,
    fetcher,
  );

  if (error) return <div>failed to load</div>;
  if (!data) return <>loading</>;
  return (
    <Container fluid>
      <Heading title={`Dashboard`}></Heading>
      <StatisticBar />
      <Grid>
        <Grid.Col span={8}>
          <Section title="Current Statistics">
            <ReportGraph data={data} height={130}></ReportGraph>
          </Section>
        </Grid.Col>
        <Grid.Col span={4}>
          <Section title={`${error_total_number} Errors`}>
            <ErrorBox></ErrorBox>
          </Section>
        </Grid.Col>
      </Grid>
    </Container>
  );
}

export default Home;
