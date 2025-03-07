import { retrieveCommodityInfo } from '@/api/requests.ts';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card.tsx';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table.tsx';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs.tsx';
import { COMMODITIES_LINK } from '@/layout/Sidebar.tsx';
import { useDocumentTitle } from '@mantine/hooks';
import { format } from 'date-fns';
import { useAtomValue, useSetAtom } from 'jotai/index';
import { useEffect } from 'react';
import { useParams } from 'react-router';
import { useAsync } from 'react-use';
import Amount from '../components/Amount';
import { breadcrumbAtom, titleAtom } from '../states/basic';

export default function SingleCommodity() {
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  let { commodityName } = useParams();

  const {
    value: commodity,
    error,
    loading,
  } = useAsync(async () => {
    const res = await retrieveCommodityInfo({ commodity_name: commodityName ?? '' });
    return res.data.data;
  }, [commodityName]);

  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`${commodityName} | Commodities - ${ledgerTitle}`);
  useEffect(() => {
    setBreadcrumb([
      COMMODITIES_LINK,
      {
        label: commodityName ?? '',
        uri: `/commodities/${commodityName}`,
        noTranslate: true,
      },
    ]);
  }, [commodityName]);
  if (error) return <div>failed to load</div>;
  if (loading || !commodity) return <div>loading</div>;

  return (
    <div>
      <div className="flex items-center gap-4 pb-6">
        <h1 className="flex-1 shrink-0 whitespace-nowrap text-xl font-semibold tracking-tight sm:grow-0">{commodityName!}</h1>
      </div>
      <Tabs defaultValue="lots">
        <TabsList>
          <TabsTrigger value="lots">Lots</TabsTrigger>
          <TabsTrigger value="price_history">Price History</TabsTrigger>
        </TabsList>

        <TabsContent value="lots">
          <Card className="mt-2 rounded-sm ">
            <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2 ">
              <CardTitle>Lots</CardTitle>
            </CardHeader>
            <CardContent>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Account</TableHead>
                    <TableHead style={{ textAlign: 'right' }}>Cost</TableHead>
                    <TableHead style={{ textAlign: 'right' }}>Price</TableHead>
                    <TableHead style={{ textAlign: 'right' }}>Balance</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {commodity.lots.map((it, idx) => (
                    <TableRow key={idx}>
                      <TableCell>{it.account}</TableCell>
                      <TableCell style={{ textAlign: 'right' }}>
                        {it.cost?.number} {it.cost?.currency}
                      </TableCell>
                      <TableCell style={{ textAlign: 'right' }}>
                        {it.price?.number} {it.price?.currency}
                      </TableCell>
                      <TableCell style={{ textAlign: 'right' }}>
                        <Amount amount={it.amount} currency={''} />
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="price_history">
          <Card className="mt-2 rounded-sm ">
            <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2 ">
              <CardTitle>Price History</CardTitle>
            </CardHeader>
            <CardContent>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Date</TableHead>
                    <TableHead>Price</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {commodity.prices.map((it, idx) => (
                    <TableRow key={idx}>
                      <TableCell>{format(new Date(it.datetime), 'yyyy-MM-dd')}</TableCell>
                      <TableCell>
                        <Amount amount={it.amount} currency={it.target_commodity ?? ''} />
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>
    </div>
  );
}
