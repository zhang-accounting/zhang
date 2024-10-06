import { useDocumentTitle, useLocalStorage } from '@mantine/hooks';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Setting } from '../components/basic/Setting';
import Section from '../components/Section';
import useSWR from 'swr';
import { fetcher } from '../global.ts';
import { Option, PluginResponse } from '../rest-model';
import PluginBox from '../components/PluginBox';
import { breadcrumbAtom, titleAtom, versionAtom } from '../states/basic';
import { useAtomValue, useSetAtom } from 'jotai/index';
import { Table, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table.tsx';
import { Skeleton } from '@/components/ui/skeleton.tsx';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select.tsx';
import { SETTINGS_LINK } from '@/layout/Sidebar.tsx';

export default function Settings() {
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const { i18n } = useTranslation();
  const [lang, setLang] = useLocalStorage({ key: 'lang', defaultValue: 'en' });
  const { data } = useSWR<Option[]>('/api/options', fetcher);
  const { data: plugins } = useSWR<PluginResponse[]>('/api/plugins', fetcher);

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
    <div className='flex flex-col gap-4'>
      <h1 className="flex-1 shrink-0 whitespace-nowrap text-xl font-semibold tracking-tight sm:grow-0">Settings</h1>
      <Section title="Basic Setting">
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
          <Setting title="title" uppercase value={ledgerTitle} />
          <Setting title="version" uppercase value={ledgerVersion} />
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
      <Section title="Plugins">
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
          {(plugins ?? []).map((plugin) => (
            <PluginBox name={plugin.name} version={plugin.version} plugin_type={plugin.plugin_type}></PluginBox>
          ))}
        </div>
      </Section>
      <Section title="Options">

        <Table >
          <TableHeader>
            <TableRow>
              <TableHead>Key</TableHead>
              <TableHead>Value</TableHead>
            </TableRow>
          </TableHeader>
          <tbody>
            {!data ? (
              <TableRow>
                <TableCell>
                  <Skeleton className="h-20 w-full" />
                </TableCell>
                <TableCell>
                  <Skeleton className="h-20 w-full" />
                </TableCell>
              </TableRow>
            ) : (
              data.map((option) => (
                <TableRow key={option.key}>
                  <TableCell className="m-1">{option.key}</TableCell>
                  <TableCell className="m-1">{option.value}</TableCell>
                </TableRow>
              ))
            )}
          </tbody>
        </Table>
      </Section>
    </div>
  );
}
