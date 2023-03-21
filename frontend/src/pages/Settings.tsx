import { Container, Grid, SegmentedControl, Title } from '@mantine/core';
import { useLocalStorage } from '@mantine/hooks';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Setting } from '../components/basic/Setting';
import Section from '../components/Section';
import { useAppSelector } from '../states';

export default function Settings() {
  const { i18n } = useTranslation();
  const [lang, setLang] = useLocalStorage({ key: 'lang', defaultValue: 'en' });
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
          <Grid.Col span={4}><Setting title="title" uppercase value={basicInfo.title} /></Grid.Col>
          <Grid.Col span={4}><Setting title="version" uppercase value={basicInfo.version} /></Grid.Col>
          <Grid.Col span={4}><Setting title="language" uppercase />
            <SegmentedControl
              value={lang}
              onChange={onLanguageChange}
              color="blue"
              data={[
                { label: '中文', value: 'zh' },
                { label: 'English', value: 'en' },
              ]} />
          </Grid.Col>
        </Grid>
      </Section>
    </Container>
  );
}
