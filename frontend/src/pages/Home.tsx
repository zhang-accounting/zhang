import { retrieveStatisticGraph } from '@/api/requests.ts';
import { DASHBOARD_LINK } from '@/layout/Sidebar.tsx';
import { useDocumentTitle } from '@mantine/hooks';
import { useAtomValue, useSetAtom } from 'jotai';
import { useEffect } from 'react';
import { useAsync } from 'react-use';
import ErrorBox from '../components/ErrorBox';
import ReportGraph from '../components/ReportGraph';
import Section from '../components/Section';
import StatisticBar from '../components/StatisticBar';
import { breadcrumbAtom, titleAtom } from '../states/basic';
import { errorCountAtom } from '../states/errors';

function Home() {
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const error_total_number = useAtomValue(errorCountAtom);
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`Dashboard - ${ledgerTitle}`);

  const now = new Date();
  const beginning_time = new Date(now.getFullYear(), now.getMonth() - 1, now.getDate(), 0, 0, 1);
  const end_time = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 23, 59, 59);

  const {
    loading,
    error,
    value: data,
  } = useAsync(async () => {
    const res = await retrieveStatisticGraph({ from: beginning_time.toISOString(), to: end_time.toISOString(), interval: 'Day' });
    return res.data.data;
  }, []);

  useEffect(() => {
    setBreadcrumb([DASHBOARD_LINK]);
  }, []);

  if (error) return <div>failed to load</div>;
  if (loading || !data) return <div>loading...</div>;
  return (
    <div className="flex flex-col gap-4">
      <StatisticBar />

      <div className="grid grid-cols-12 gap-4">
        <div className="col-span-12 lg:col-span-8">
          <Section title="Current Statistics">
            <ReportGraph data={data} height={130}></ReportGraph>
          </Section>
        </div>
        <div className="col-span-12 lg:col-span-4">
          <Section title={`${error_total_number} Errors`}>
            <ErrorBox></ErrorBox>
          </Section>
        </div>
      </div>
    </div>
  );
}

export default Home;
