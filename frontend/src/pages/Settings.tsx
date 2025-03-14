import { retrieveOptions, retrievePlugins } from '@/api/requests.ts';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select.tsx';
import { Table, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table.tsx';
import { SETTINGS_LINK } from '@/layout/Sidebar.tsx';
import { useDocumentTitle, useLocalStorage } from '@mantine/hooks';
import { useAtomValue, useSetAtom } from 'jotai/index';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { useAsync } from 'react-use';
import { Setting } from '../components/basic/Setting';
import PluginBox from '../components/PluginBox';
import Section from '../components/Section';
import { breadcrumbAtom, titleAtom, versionAtom } from '../states/basic';
import PwaInstallBanner from '../components/PwaInstallBanner';
import { Button } from '@/components/ui/button';
import { ExternalLink } from 'lucide-react';

export default function Settings() {
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const { i18n, t } = useTranslation();
  const [lang, setLang] = useLocalStorage({ key: 'lang', defaultValue: 'en' });

  const { value: options } = useAsync(async () => {
    const res = await retrieveOptions({});
    return res.data.data;
  }, []);
  const { value: plugins } = useAsync(async () => {
    const res = await retrievePlugins({});
    return res.data.data;
  }, []);

  const ledgerTitle = useAtomValue(titleAtom);
  const ledgerVersion = useAtomValue(versionAtom);

  useDocumentTitle(`Settings - ${ledgerTitle}`);
  useEffect(() => {
    setBreadcrumb([SETTINGS_LINK]);
  }, []);

  const onLanguageChange = (lang: string) => {
    setLang(lang);
  };

  useEffect(() => {
    i18n.changeLanguage(lang);
  }, [lang, i18n]);

  return (
    <div className="container mx-auto px-4 py-8">
      <h1 className="text-2xl font-bold mb-6">{t('settings.title')}</h1>

      <div className="space-y-6">
        {/* PWA 安装横幅 */}
        <PwaInstallBanner />

        <Section title="Basic Setting">
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
            <Setting title="title" uppercase value={ledgerTitle ?? ''} />
            <Setting title="version" uppercase value={ledgerVersion ?? ''} />
            <div>
              <Setting title="language" uppercase />
              <div className="mt-2">
                <Select value={lang} onValueChange={onLanguageChange}>
                  <SelectTrigger className="w-[180px]">
                    <SelectValue placeholder="Select a fruit" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="zh">中文</SelectItem>
                    <SelectItem value="en">English</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>
          </div>
        </Section>

        <Section title="OpenAPI Documentation">
          <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
            <Button variant="outline" className="flex items-center gap-2" onClick={() => window.open('/openapi.json', '_blank')}>
              OpenAPI JSON
              <ExternalLink className="h-4 w-4" />
            </Button>
            <Button variant="outline" className="flex items-center gap-2" onClick={() => window.open('/swagger', '_blank')}>
              Swagger UI
              <ExternalLink className="h-4 w-4" />
            </Button>
            <Button variant="outline" className="flex items-center gap-2" onClick={() => window.open('/scalar', '_blank')}>
              Scalar
              <ExternalLink className="h-4 w-4" />
            </Button>
          </div>
        </Section>

        <Section title="Plugins">
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
            {(plugins ?? []).map((plugin) => (
              <PluginBox key={plugin.name} name={plugin.name} version={plugin.version} plugin_type={plugin.plugin_type}></PluginBox>
            ))}
          </div>
        </Section>
        <Section title="Options">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Key</TableHead>
                <TableHead>Value</TableHead>
              </TableRow>
            </TableHeader>
            <tbody>
              {(options ?? []).map((option) => (
                <TableRow key={option.key}>
                  <TableCell className="m-1">{option.key}</TableCell>
                  <TableCell className="m-1">{option.value}</TableCell>
                </TableRow>
              ))}
            </tbody>
          </Table>
        </Section>
      </div>
    </div>
  );
}
