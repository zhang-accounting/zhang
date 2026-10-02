import BigNumber from 'bignumber.js';
import { format } from 'date-fns';
import { useAtomValue, useSetAtom } from 'jotai';
import { ArrowLeft, ChartLine, Layers, ListX, TriangleAlert } from 'lucide-react';
import { OpReturnType } from 'openapi-typescript-fetch';
import { useEffect, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { Link, useParams } from 'react-router-dom';
import { useAsync } from 'react-use';
import { CartesianGrid, Line, LineChart, XAxis, YAxis } from 'recharts';
import { retrieveCommodityInfo } from '@/api/requests';
import { operations } from '@/api/schemas';
import Amount from '@/components/Amount';
import { CommodityLatestPrice } from '@/components/CommodityBox';
import { EmptyState, PageHeader, PageShell, ResponsiveList, type ResponsiveColumn } from '@/components/layout';
import { KeyFigure, KeyFigures } from '@/components/layout/KeyFigures';
import { Badge } from '@/components/ui/badge';
import { buttonVariants } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { ChartConfig, ChartContainer, ChartTooltip, ChartTooltipContent } from '@/components/ui/chart';
import { Skeleton } from '@/components/ui/skeleton';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { useDocumentTitle } from '@/hooks/use-document-title';
import { COMMODITIES_LINK } from '@/layout/nav-links';
import { cn } from '@/lib/utils';
import { breadcrumbAtom, titleAtom } from '@/states/basic';

type CommodityDetail = OpReturnType<operations['get_single_commodity']>['data'];
type Lot = CommodityDetail['lots'][number];
type Price = CommodityDetail['prices'][number];

/** One quote commodity: the brand primary. Several: chart-1..5 in a fixed order (never cycled), any further ones muted. */
const SERIES_COLORS = ['var(--chart-1)', 'var(--chart-2)', 'var(--chart-3)', 'var(--chart-4)', 'var(--chart-5)'];
const seriesColor = (index: number, count: number) => (count === 1 ? 'var(--primary)' : (SERIES_COLORS[index] ?? 'var(--muted-foreground)'));

function UnitPrice({ unit }: { unit?: { number: string; commodity: string } | null }) {
  if (!unit) return <span className="text-muted-foreground">—</span>;
  return <Amount amount={unit.number} currency={unit.commodity} />;
}

/** Price history as one line per quote commodity. */
function PriceHistoryChart({ prices }: { prices: Price[] }) {
  const { series, data, config } = useMemo(() => {
    const quoteCommodities = Array.from(new Set(prices.map((price) => price.amount.commodity))).sort();
    const byDate = new Map<string, Record<string, number | string>>();
    [...prices]
      .sort((a, b) => new Date(a.datetime).getTime() - new Date(b.datetime).getTime())
      .forEach((price) => {
        const key = format(new Date(price.datetime), 'yyyy-MM-dd');
        const row = byDate.get(key) ?? { date: key };
        row[price.amount.commodity] = new BigNumber(price.amount.number).toNumber();
        byDate.set(key, row);
      });
    const chartConfig = quoteCommodities.reduce<ChartConfig>(
      (acc, commodity, index) => ({ ...acc, [commodity]: { label: commodity, color: seriesColor(index, quoteCommodities.length) } }),
      {},
    );
    return { series: quoteCommodities, data: Array.from(byDate.values()), config: chartConfig };
  }, [prices]);

  return (
    <ChartContainer config={config} className="aspect-auto h-56 w-full md:h-72">
      <LineChart accessibilityLayer data={data} margin={{ left: 4, right: 12, top: 8 }}>
        <CartesianGrid vertical={false} />
        <XAxis dataKey="date" tickLine={false} axisLine={false} tickMargin={8} minTickGap={24} tickFormatter={(value: string) => value.slice(5)} />
        <YAxis tickLine={false} axisLine={false} width={48} domain={['auto', 'auto']} tickMargin={4} />
        <ChartTooltip cursor={false} content={<ChartTooltipContent />} />
        {series.map((commodity) => (
          <Line
            key={commodity}
            dataKey={commodity}
            type="monotone"
            stroke={config[commodity]?.color}
            strokeWidth={2}
            dot={data.length <= 12}
            isAnimationActive={false}
            connectNulls
          />
        ))}
      </LineChart>
    </ChartContainer>
  );
}

export default function SingleCommodity() {
  const { t } = useTranslation();
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const { commodityName } = useParams();
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`${commodityName} | ${t('NAV_COMMODITIES')} - ${ledgerTitle}`);
  useEffect(() => {
    setBreadcrumb([COMMODITIES_LINK, { label: commodityName ?? '', uri: `/commodities/${commodityName}`, noTranslate: true }]);
  }, [commodityName, setBreadcrumb]);

  const {
    value: commodity,
    error,
    loading,
  } = useAsync(async () => {
    const res = await retrieveCommodityInfo({ commodity_name: commodityName ?? '' });
    return res.data.data;
  }, [commodityName]);

  const lotColumns: ResponsiveColumn<Lot>[] = [
    {
      key: 'account',
      header: t('commodities.account'),
      cell: (lot) => <span className="font-medium">{lot.account}</span>,
    },
    {
      key: 'acquired',
      header: t('commodities.acquired'),
      className: 'w-32 text-muted-foreground tabular-nums',
      cell: (lot) => lot.acquisition_date ?? '—',
    },
    { key: 'cost', header: t('commodities.cost'), className: 'text-right tabular-nums', cell: (lot) => <UnitPrice unit={lot.cost} /> },
    { key: 'price', header: t('commodities.price'), className: 'text-right tabular-nums', cell: (lot) => <UnitPrice unit={lot.price} /> },
    {
      key: 'amount',
      header: t('commodities.amount'),
      className: 'w-44 text-right font-medium tabular-nums',
      cell: (lot) => <Amount amount={lot.amount} currency={commodityName ?? ''} />,
    },
  ];

  const priceColumns: ResponsiveColumn<Price>[] = [
    { key: 'date', header: t('commodities.date'), className: 'tabular-nums', cell: (price) => format(new Date(price.datetime), 'yyyy-MM-dd') },
    {
      key: 'price',
      header: t('commodities.price'),
      className: 'text-right font-medium tabular-nums',
      cell: (price) => <Amount amount={price.amount.number} currency={price.amount.commodity} />,
    },
  ];

  if (error) {
    return (
      <PageShell>
        <PageHeader title={commodityName} />
        <EmptyState
          icon={TriangleAlert}
          title={t('commodities.not_found_title')}
          description={error.message}
          action={
            <Link to="/commodities" className={cn(buttonVariants({ variant: 'outline' }), 'h-10 md:h-8')}>
              <ArrowLeft />
              {t('commodities.back')}
            </Link>
          }
        />
      </PageShell>
    );
  }

  const info = commodity?.info;
  const lots = commodity?.lots ?? [];
  const prices = commodity?.prices ?? [];
  const sortedPrices = [...prices].sort((a, b) => new Date(b.datetime).getTime() - new Date(a.datetime).getTime());
  const firstLoad = loading && commodity === undefined;

  return (
    <PageShell>
      <PageHeader title={commodityName} description={t('commodities.detail_description')}>
        {info && (info.group || info.prefix || info.suffix) && (
          <div className="flex flex-wrap gap-2">
            {(info.prefix || info.suffix) && <Badge variant="outline">{info.prefix || info.suffix}</Badge>}
            {info.group && <Badge variant="secondary">{info.group}</Badge>}
          </div>
        )}
      </PageHeader>

      <KeyFigures>
        <KeyFigure loading={firstLoad} label={t('commodities.holdings')} value={info && <Amount amount={info.total_amount} currency={info.name} />} />
        <KeyFigure
          loading={firstLoad}
          label={t('commodities.latest_price')}
          value={info && <CommodityLatestPrice {...info} />}
          hint={info?.latest_price_date ? format(new Date(info.latest_price_date), 'yyyy-MM-dd') : undefined}
        />
        <KeyFigure loading={firstLoad} label={t('commodities.lots')} value={lots.length} />
        <KeyFigure loading={firstLoad} label={t('commodities.precision')} value={info?.precision} hint={info?.rounding} />
      </KeyFigures>

      <Card>
        <CardHeader>
          <CardTitle>{t('commodities.price_history')}</CardTitle>
          <CardDescription>{t('commodities.price_history_description', { count: prices.length })}</CardDescription>
        </CardHeader>
        <CardContent>
          {firstLoad ? (
            <Skeleton className="h-56 w-full md:h-72" />
          ) : prices.length === 0 ? (
            <EmptyState icon={ChartLine} title={t('commodities.no_prices_title')} description={t('commodities.no_prices_description')} className="py-10" />
          ) : (
            <PriceHistoryChart prices={prices} />
          )}
        </CardContent>
      </Card>

      <Tabs defaultValue="lots">
        <TabsList>
          <TabsTrigger value="lots" className="px-3">
            {t('commodities.lots')}
            <span className="text-muted-foreground tabular-nums">{lots.length}</span>
          </TabsTrigger>
          <TabsTrigger value="prices" className="px-3">
            {t('commodities.price_records')}
            <span className="text-muted-foreground tabular-nums">{prices.length}</span>
          </TabsTrigger>
        </TabsList>
        <TabsContent value="lots" className="pt-2">
          <ResponsiveList
            items={lots}
            loading={firstLoad}
            getKey={(lot, index) => `${lot.account}-${index}`}
            getItemHref={(lot) => `/accounts/${lot.account}`}
            columns={lotColumns}
            renderCard={(lot) => (
              <div className="flex flex-col gap-1">
                <div className="flex items-start justify-between gap-3">
                  <span className="min-w-0 truncate font-medium">{lot.account}</span>
                  <span className="shrink-0 font-medium tabular-nums">
                    <Amount amount={lot.amount} currency={commodityName ?? ''} />
                  </span>
                </div>
                {(lot.cost || lot.price || lot.acquisition_date) && (
                  <div className="flex flex-wrap gap-x-3 text-xs text-muted-foreground tabular-nums">
                    {lot.acquisition_date && <span>{lot.acquisition_date}</span>}
                    {lot.cost && (
                      <span>
                        {t('commodities.cost')} <UnitPrice unit={lot.cost} />
                      </span>
                    )}
                    {lot.price && (
                      <span>
                        {t('commodities.price')} <UnitPrice unit={lot.price} />
                      </span>
                    )}
                  </div>
                )}
              </div>
            )}
            empty={<EmptyState icon={Layers} title={t('commodities.no_lots_title')} description={t('commodities.no_lots_description')} />}
          />
        </TabsContent>
        <TabsContent value="prices" className="pt-2">
          <ResponsiveList
            items={sortedPrices}
            loading={firstLoad}
            getKey={(price, index) => `${price.datetime}-${index}`}
            columns={priceColumns}
            renderCard={(price) => (
              <div className="flex items-center justify-between gap-3">
                <span className="text-muted-foreground tabular-nums">{format(new Date(price.datetime), 'yyyy-MM-dd')}</span>
                <span className="font-medium tabular-nums">
                  <Amount amount={price.amount.number} currency={price.amount.commodity} />
                </span>
              </div>
            )}
            empty={<EmptyState icon={ListX} title={t('commodities.no_prices_title')} description={t('commodities.no_prices_description')} />}
          />
        </TabsContent>
      </Tabs>
    </PageShell>
  );
}
