import { Container, Grid, SegmentedControl, SimpleGrid, Skeleton, Table } from '@mantine/core';
import { useDocumentTitle, useLocalStorage } from '@mantine/hooks';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Setting } from '../components/basic/Setting';
import Section from '../components/Section';
import { useAppSelector } from '../states';
import useSWR from 'swr';
import { fetcher } from '..';
import { Option, PluginResponse } from '../rest-model';
import { Heading } from '../components/basic/Heading';
import PluginBox from '../components/PluginBox';

export default function Settings() {
  const { i18n } = useTranslation();
  const [lang, setLang] = useLocalStorage({ key: 'lang', defaultValue: 'en' });
  const { data } = useSWR<Option[]>('/api/options', fetcher);
  const { data: plugins } = useSWR<PluginResponse[]>('/api/plugins', fetcher);

  const ledgerTitle = useAppSelector((state) => state.basic.title ?? 'Zhang Accounting');

  useDocumentTitle(`Settings - ${ledgerTitle}`);

  const onLanguageChange = (lang: string) => {
    setLang(lang);
  };

  useEffect(() => {
    i18n.changeLanguage(lang);
  }, [lang, i18n]);

  const basicInfo = useAppSelector((state) => state.basic);

  return (
    <Container fluid>
      <Heading title={`Settings`}></Heading>
      <Section title="Basic Setting">
        <Grid>
          <Grid.Col span={{ base: 4, md: 6, sm: 12 }}>
            <Setting title="title" uppercase value={basicInfo.title} />
          </Grid.Col>
          <Grid.Col span={{ base: 4, md: 6, sm: 12 }}>
            <Setting title="version" uppercase value={basicInfo.version} />
          </Grid.Col>
          <Grid.Col span={{ base: 4, md: 6, sm: 12 }}>
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
      <Section title="Plugins">
        <SimpleGrid cols={2}>
          {(plugins ?? []).map((plugin) => (
            <PluginBox name={plugin.name} version={plugin.version} plugin_type={plugin.plugin_type}></PluginBox>
          ))}
        </SimpleGrid>
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
              <tr key={option.key}>
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
