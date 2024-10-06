import useSWR from 'swr';
import ErrorBox from '../components/ErrorBox';
import Section from '../components/Section';
import StatisticBar from '../components/StatisticBar';
import { StatisticGraphResponse } from '../rest-model';
import ReportGraph from '../components/ReportGraph';
import { useDocumentTitle } from '@mantine/hooks';
import { useAtomValue, useSetAtom } from 'jotai';
import { errorCountAtom } from '../states/errors';
import { breadcrumbAtom, titleAtom } from '../states/basic';
import { fetcher } from '../global.ts';
import { useEffect } from 'react';
import { DASHBOARD_LINK } from '@/layout/Sidebar.tsx';

function Home() {
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
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

  useEffect(() => {
    setBreadcrumb([DASHBOARD_LINK]);
  }, []);

  if (error) return <div>failed to load</div>;
  if (!data) return <>loading</>;
  return (
    <div className="flex flex-col gap-4">
      <StatisticBar />

      <div className="grid grid-cols-12 gap-4">
        <div className="col-span-8">
          <Section title="Current Statistics">
            <ReportGraph data={data} height={130}></ReportGraph>
          </Section>
        </div>
        <div className="col-span-4">
          <Section title={`${error_total_number} Errors`}>
            <ErrorBox></ErrorBox>
          </Section>
        </div>
      </div>
    </div>
  );
}

export default Home;
