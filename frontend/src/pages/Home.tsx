import { Container, Grid, Title } from '@mantine/core';
import { useTranslation } from 'react-i18next';

import useSWR from 'swr';
import ErrorBox from '../components/ErrorBox';
import Section from '../components/Section';
import StatisticBar from '../components/StatisticBar';
import { fetcher } from '../index';
import { StatisticResponse } from '../rest-model';
import { useAppSelector } from '../states';
import ReportGraph from '../components/ReportGraph';

function Home() {
  const { t } = useTranslation();
  const error_total_number = useAppSelector((state) => state.errors.total_number);
  const now = new Date();
  const beginning_time = new Date(now.getFullYear(), now.getMonth() - 1, now.getDate(), 0, 0, 1);
  const end_time = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 23, 59, 59);

  const { data, error } = useSWR<StatisticResponse>(`/api/statistic?from=${beginning_time.toISOString()}&to=${end_time.toISOString()}&interval=Day`, fetcher);

  if (error) return <div>failed to load</div>;
  if (!data) return <>loading</>;
  return (
    <Container fluid>
      <Title order={2}>{t('Dashboard')}</Title>
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
