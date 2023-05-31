import { Container, Grid, SegmentedControl, Skeleton, Table, Title } from '@mantine/core';
import { useLocalStorage } from '@mantine/hooks';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Setting } from '../components/basic/Setting';
import Section from '../components/Section';
import { useAppSelector } from '../states';
import useSWR from 'swr';
import { fetcher } from '..';
import { Option } from '../rest-model';

export default function Settings() {
  const { i18n } = useTranslation();
  const [lang, setLang] = useLocalStorage({ key: 'lang', defaultValue: 'en' });
  const { data } = useSWR<Option[]>('/api/options', fetcher);

  const onLanguageChange = (lang: string) => {
    setLang(lang);
  };

  useEffect(() => {
    i18n.changeLanguage(lang);
  }, [lang, i18n]);

  const basicInfo = useAppSelector((state) => state.basic);

  return (
    <Container fluid>
      <Title order={2}>Settings</Title>

      <Section title="Basic Setting">
        <Grid>
          <Grid.Col sm={12} md={6} lg={4}>
            <Setting title="title" uppercase value={basicInfo.title} />
          </Grid.Col>
          <Grid.Col sm={12} md={6} lg={4}>
            <Setting title="version" uppercase value={basicInfo.version} />
          </Grid.Col>
          <Grid.Col sm={12} md={6} lg={4}>
            <Setting title="language" uppercase />
            <SegmentedControl
              value={lang}
              onChange={onLanguageChange}
              color="blue"
              data={[
                { label: '中文', value: 'zh' },
                { label: 'English', value: 'en' },
              ]}
            />
          </Grid.Col>
        </Grid>
      </Section>
      <Section title="Options">
        <Table verticalSpacing="xs" highlightOnHover>
          <thead>
            <tr>
              <th>Key</th>
              <th>Value</th>
            </tr>
          </thead>
          <tbody>
            {!data ? (
              <tr>
                <td>
                  <Skeleton height={20} mt={10} radius="xs" />
                </td>
                <td>
                  <Skeleton height={20} mt={10} radius="xs" />
                </td>
              </tr>
            ) : (
              data.map((option) => (
                <tr>
                  <td>{option.key}</td>
                  <td>{option.value}</td>
                </tr>
              ))
            )}
          </tbody>
        </Table>
      </Section>
    </Container>
  );
}
