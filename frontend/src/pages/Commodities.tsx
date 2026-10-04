import { format } from 'date-fns';
import { useAtomValue, useSetAtom } from 'jotai';
import { ChevronRight, Coins } from 'lucide-react';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { useAsync } from 'react-use';
import { retrieveOptions } from '@/api/requests';
import Amount from '@/components/Amount';
import CommodityBox, { CommodityLatestPrice, CommoditySymbol, type CommodityBoxProps } from '@/components/CommodityBox';
import { EmptyState, LoadFailedState, PageHeader, PageShell, ResponsiveList, type ResponsiveColumn } from '@/components/layout';
import { OpenInExplore } from '@/components/query/OpenInExplore';
import { Badge } from '@/components/ui/badge';
import { useDocumentTitle } from '@/hooks/use-document-title';
import { COMMODITIES_LINK } from '@/layout/nav-links';
import { breadcrumbAtom, titleAtom } from '@/states/basic';
import { commoditiesAtom, commoditiesFetcher, FRONTEND_DEFAULT_GROUP, groupedCommoditiesAtom } from '@/states/commodity';

type CommodityRow = CommodityBoxProps;

export default function Commodities() {
  const { t } = useTranslation();
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`${t('NAV_COMMODITIES')} - ${ledgerTitle}`);
  useEffect(() => {
    setBreadcrumb([COMMODITIES_LINK]);
  }, [setBreadcrumb]);

  const commodities = useAtomValue(commoditiesAtom);
  const refreshCommodities = useSetAtom(commoditiesFetcher);
  const groupedCommodities = useAtomValue(groupedCommoditiesAtom);
  const { value: operatingCurrency } = useAsync(async () => {
    const res = await retrieveOptions({});
    return res.data.data.find((option) => option.key === 'operating_currency')?.value;
  }, []);

  const groupNames = Object.keys(groupedCommodities).sort((a, b) =>
    a === FRONTEND_DEFAULT_GROUP ? -1 : b === FRONTEND_DEFAULT_GROUP ? 1 : a.localeCompare(b),
  );
  const toRows = (group: string): CommodityRow[] =>
    [...groupedCommodities[group]]
      .sort((a, b) => a.name.localeCompare(b.name))
      .map((commodity) => ({ ...commodity, operating_currency: commodity.name === operatingCurrency }));

  const columns: ResponsiveColumn<CommodityRow>[] = [
    {
      key: 'name',
      header: t('commodities.commodity'),
      cell: (row) => (
        <div className="flex items-center gap-3">
          <CommoditySymbol name={row.name} prefix={row.prefix} suffix={row.suffix} className="size-8" />
          <span className="font-medium">{row.name}</span>
          {row.operating_currency && <Badge variant="secondary">{t('commodities.operating')}</Badge>}
        </div>
      ),
    },
    {
      key: 'price',
      header: t('commodities.latest_price'),
      className: 'text-right',
      cell: (row) => <CommodityLatestPrice {...row} />,
    },
    {
      key: 'price_date',
      header: t('commodities.price_date'),
      className: 'w-32 text-right text-muted-foreground tabular-nums',
      cell: (row) => (row.latest_price_date ? format(new Date(row.latest_price_date), 'yyyy-MM-dd') : '—'),
    },
    {
      key: 'holdings',
      header: t('commodities.holdings'),
      className: 'w-48 text-right font-medium tabular-nums',
      cell: (row) => <Amount amount={row.total_amount} currency={row.name} />,
    },
    {
      key: 'chevron',
      header: <span className="sr-only">{t('commodities.open')}</span>,
      className: 'w-8',
      cell: () => <ChevronRight className="size-4 text-muted-foreground" />,
    },
  ];

  const loading = commodities.state === 'loading';
  const showGroupTitles = groupNames.length > 1;

  return (
    <PageShell>
      <PageHeader
        title={t('NAV_COMMODITIES')}
        description={t('commodities.description')}
        actions={<OpenInExplore name="commodities.totals" iconOnly className="size-10 md:size-8" />}
      />
      {commodities.state === 'hasError' ? (
        <LoadFailedState onRetry={refreshCommodities} />
      ) : loading || groupNames.length === 0 ? (
        <ResponsiveList
          items={[] as CommodityRow[]}
          loading={loading}
          getKey={(row) => row.name}
          columns={columns}
          renderCard={(row) => <CommodityBox {...row} />}
          empty={<EmptyState icon={Coins} title={t('commodities.empty_title')} description={t('commodities.empty_description')} />}
        />
      ) : (
        groupNames.map((group) => (
          <section key={group} className="flex flex-col gap-3">
            {showGroupTitles && (
              <h2 className="text-sm font-semibold text-muted-foreground">{group === FRONTEND_DEFAULT_GROUP ? t('commodities.default_group') : group}</h2>
            )}
            <ResponsiveList
              items={toRows(group)}
              getKey={(row) => row.name}
              columns={columns}
              renderCard={(row) => <CommodityBox {...row} />}
              getItemHref={(row) => `/commodities/${encodeURIComponent(row.name)}`}
            />
          </section>
        ))
      )}
    </PageShell>
  );
}
