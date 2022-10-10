import { Container, Title, SegmentedControl } from '@mantine/core';
import { useLocalStorage } from '@mantine/hooks';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';

export default function Settings() {
  const { i18n } = useTranslation();
  const [lang, setLang] = useLocalStorage({ key: 'lang', defaultValue: 'en' });
  const onLanguageChange = (lang: string) => {
    setLang(lang);
  };
  useEffect(() => {
    i18n.changeLanguage(lang);
  }, [lang]);

  return (
    <Container fluid>
      <Title order={2}>Settings</Title>

      <SegmentedControl
        value={lang}
        onChange={onLanguageChange}
        color="blue"
        data={[
          { label: '中文', value: 'zh' },
          { label: 'English', value: 'en' },
        ]}
      />
    </Container>
  );
}
