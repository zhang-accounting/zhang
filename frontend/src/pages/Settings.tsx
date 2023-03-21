import { Container, Grid, SegmentedControl, Title } from '@mantine/core';
import { useLocalStorage } from '@mantine/hooks';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
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

      <Grid>
        <Grid.Col span={4}>title</Grid.Col>
        <Grid.Col span={8}>{basicInfo.title}</Grid.Col>
        <Grid.Col span={4}>version</Grid.Col>
        <Grid.Col span={8}>{basicInfo.version}</Grid.Col>
        <Grid.Col span={4}>language</Grid.Col>
        <Grid.Col span={8}>
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
    </Container>
  );
}
