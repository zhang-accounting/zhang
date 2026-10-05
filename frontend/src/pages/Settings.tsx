import { useAtomValue, useSetAtom } from 'jotai';
import { ArrowUpRight, ExternalLink, Puzzle, RotateCw } from 'lucide-react';
import { useTheme } from 'next-themes';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { useAsync } from 'react-use';
import { serverBaseUrl } from '@/api/fetcher';
import { retrievePlugins } from '@/api/requests';
import { PasskeySettings } from '@/components/auth/PasskeySettings';
import { SettingRow, SettingsSection } from '@/components/basic/Setting';
import { EmptyState, PageHeader, PageShell } from '@/components/layout';
import PluginBox from '@/components/PluginBox';
import PwaInstallBanner from '@/components/PwaInstallBanner';
import { Badge } from '@/components/ui/badge';
import { Button, buttonVariants } from '@/components/ui/button';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Skeleton } from '@/components/ui/skeleton';
import { useDocumentTitle } from '@/hooks/use-document-title';
import { useLanguage } from '@/hooks/use-language';
import { GITHUB_REPO_URL, SETTINGS_LINK, UPGRADE_GUIDE_URL } from '@/layout/nav-links';
import { THEMES } from '@/layout/themes';
import { useReloadLedger } from '@/layout/use-reload-ledger';
import { LANGUAGES } from '@/lib/languages';
import { cn } from '@/lib/utils';
import { loadable_unwrap } from '@/states';
import { basicInfoAtom, breadcrumbAtom, titleAtom, updatableVersionAtom, versionAtom } from '@/states/basic';
import { operatingCurrencyAtom, optionsAtom } from '@/states/options';

const API_DOCS = [
  { label: 'OpenAPI JSON', path: '/openapi.json' },
  { label: 'Swagger UI', path: '/swagger' },
  { label: 'Scalar', path: '/scalar' },
];

export default function Settings() {
  const { t } = useTranslation();
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const [lang, setLang] = useLanguage();
  const { theme, setTheme } = useTheme();
  const reloadLedger = useReloadLedger();

  const optionsLoadable = useAtomValue(optionsAtom);
  const optionsLoading = optionsLoadable.state === 'loading';
  const options = loadable_unwrap(optionsLoadable, undefined, (data) => data);
  const operatingCurrency = useAtomValue(operatingCurrencyAtom);
  const { value: plugins, loading: pluginsLoading } = useAsync(async () => {
    const res = await retrievePlugins({});
    return res.data.data;
  }, []);

  const ledgerTitle = useAtomValue(titleAtom);
  const ledgerVersion = useAtomValue(versionAtom);
  const basicInfo = useAtomValue(basicInfoAtom);
  const updatableVersion = useAtomValue(updatableVersionAtom);
  const buildDate = basicInfo.state === 'hasData' ? basicInfo.data.build_date : undefined;

  useDocumentTitle(`${t('settings.title')} - ${ledgerTitle}`);
  useEffect(() => {
    setBreadcrumb([SETTINGS_LINK]);
  }, [setBreadcrumb]);

  return (
    <PageShell width="narrow">
      <PageHeader title={t('settings.title')} description={t('settings.description')} />

      <PwaInstallBanner />

      <SettingsSection title={t('settings.general')}>
        <SettingRow label={t('SHELL_LANGUAGE')} description={t('settings.language_description')} htmlFor="settings-language">
          <Select items={LANGUAGES} value={lang} onValueChange={(value) => value && setLang(value)}>
            <SelectTrigger id="settings-language" className="h-10 w-full sm:w-44 md:h-8">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {LANGUAGES.map((language) => (
                <SelectItem key={language.value} value={language.value}>
                  {language.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingRow>
        <SettingRow label={t('SHELL_THEME')} description={t('settings.theme_description')}>
          <div role="radiogroup" aria-label={t('SHELL_THEME')} className="flex w-full gap-1 rounded-lg bg-muted p-1 sm:w-auto">
            {THEMES.map((item) => {
              const active = (theme ?? 'system') === item.value;
              return (
                <Button
                  key={item.value}
                  role="radio"
                  aria-checked={active}
                  variant="ghost"
                  className={cn('h-10 flex-1 px-3 md:h-7', active && 'bg-background shadow-xs hover:bg-background dark:bg-input/40')}
                  onClick={() => setTheme(item.value)}
                >
                  <item.icon />
                  {t(item.label)}
                </Button>
              );
            })}
          </div>
        </SettingRow>
      </SettingsSection>

      <PasskeySettings />

      <SettingsSection
        title={t('settings.ledger')}
        action={
          <Button variant="outline" className="h-10 md:h-8" onClick={reloadLedger}>
            <RotateCw />
            {t('SHELL_RELOAD_LEDGER')}
          </Button>
        }
      >
        <SettingRow inline label={t('settings.ledger_title')} description={t('settings.ledger_title_description')}>
          <span className="max-w-48 truncate text-sm font-medium sm:max-w-72">{ledgerTitle}</span>
        </SettingRow>
        <SettingRow inline label={t('settings.operating_currency')} description={t('settings.operating_currency_description')}>
          {optionsLoading ? (
            <Skeleton className="h-5 w-12" />
          ) : (
            <Badge variant="secondary" className="font-mono">
              {operatingCurrency ?? '—'}
            </Badge>
          )}
        </SettingRow>
        <SettingRow inline label={t('settings.version')} description={buildDate ? t('settings.build_date', { date: buildDate }) : undefined}>
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-mono text-sm tabular-nums">{ledgerVersion ?? '—'}</span>
            {updatableVersion && (
              <a href={UPGRADE_GUIDE_URL} target="_blank" rel="noreferrer" className={cn(buttonVariants({ variant: 'outline', size: 'sm' }), 'h-10 md:h-7')}>
                {t('SHELL_UPDATE_AVAILABLE', { version: updatableVersion })}
                <ArrowUpRight />
              </a>
            )}
          </div>
        </SettingRow>
      </SettingsSection>

      <SettingsSection title={t('settings.github_repository')} description={t('settings.github_repository_description')} bare>
        <a href={GITHUB_REPO_URL} target="_blank" rel="noreferrer" className={cn(buttonVariants({ variant: 'outline' }), 'h-10 justify-between md:h-8')}>
          zhang-accounting/zhang
          <ExternalLink />
        </a>
      </SettingsSection>

      <SettingsSection title={t('settings.options')} description={t('settings.options_description')}>
        {optionsLoading ? (
          <div className="flex flex-col gap-2 p-4">
            <Skeleton className="h-4 w-2/3" />
            <Skeleton className="h-4 w-1/2" />
          </div>
        ) : (
          <dl className="divide-y">
            {(options ?? []).map((option) => (
              <div key={option.key} className="grid gap-1 px-4 py-2.5 sm:grid-cols-[minmax(0,16rem)_minmax(0,1fr)] sm:gap-4">
                <dt className="truncate font-mono text-xs leading-5 text-muted-foreground">{option.key}</dt>
                <dd className="text-sm break-all">{option.value}</dd>
              </div>
            ))}
          </dl>
        )}
      </SettingsSection>

      <SettingsSection title={t('settings.plugins')} description={t('settings.plugins_description')} bare>
        {pluginsLoading ? (
          <Skeleton className="h-16 w-full rounded-xl" />
        ) : (plugins ?? []).length === 0 ? (
          <EmptyState icon={Puzzle} title={t('settings.no_plugins_title')} description={t('settings.no_plugins_description')} className="py-8" />
        ) : (
          <div className="grid gap-3 sm:grid-cols-2">
            {/* declaration order; a plugin declared twice is listed twice */}
            {(plugins ?? []).map((plugin, index) => (
              <PluginBox key={index} name={plugin.name} version={plugin.version} plugin_type={plugin.plugin_type} route={plugin.route} />
            ))}
          </div>
        )}
      </SettingsSection>

      <SettingsSection title={t('settings.api_docs')} description={t('settings.api_docs_description')} bare>
        <div className="grid gap-2 sm:grid-cols-3">
          {API_DOCS.map((doc) => (
            <a
              key={doc.path}
              href={`${serverBaseUrl}${doc.path}`}
              target="_blank"
              rel="noreferrer"
              className={cn(buttonVariants({ variant: 'outline' }), 'h-10 justify-between md:h-8')}
            >
              {doc.label}
              <ExternalLink />
            </a>
          ))}
        </div>
      </SettingsSection>
    </PageShell>
  );
}
