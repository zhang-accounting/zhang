import { retrieveAccountBalance, retrieveAccountDocuments, retrieveAccountInfo, retrieveAccountJournals } from '@/api/requests.ts';
import { Badge } from '@/components/ui/badge.tsx';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card.tsx';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table.tsx';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs.tsx';
import { ACCOUNTS_LINK } from '@/layout/Sidebar.tsx';
import { useDocumentTitle } from '@mantine/hooks';
import { format } from 'date-fns';
import { useAtomValue, useSetAtom } from 'jotai/index';
import { Cog, FileStack, Notebook } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useParams } from 'react-router';
import { useAsync } from 'react-use';
import AccountBalanceCheckLine from '../components/AccountBalanceCheckLine';
import { AccountBalanceHistoryGraph } from '../components/AccountBalanceHistoryGraph';
import AccountDocumentUpload from '../components/AccountDocumentUpload';
import Amount from '../components/Amount';
import LoadingComponent from '../components/basic/LoadingComponent';
import PayeeNarration from '../components/basic/PayeeNarration';
import { ImageLightBox } from '../components/ImageLightBox';
import DocumentPreview from '../components/journalPreview/DocumentPreview';
import { breadcrumbAtom, titleAtom } from '../states/basic';

function SingleAccount() {
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  let { accountName } = useParams();

  const [lightboxSrc, setLightboxSrc] = useState<string | undefined>(undefined);

  const { value: account, error } = useAsync(async () => {
    if (!accountName) return undefined;
    const res = await retrieveAccountInfo({ account_name: accountName });
    return res.data.data;
  }, [accountName]);

  const { value: account_balance_data, error: account_balance_error } = useAsync(async () => {
    if (!accountName) return undefined;
    const res = await retrieveAccountBalance({ account_name: accountName });
    return res.data.data;
  }, [accountName]);

  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`${accountName} | Accounts - ${ledgerTitle}`);

  useEffect(() => {
    setBreadcrumb([
      ACCOUNTS_LINK,
      {
        label: accountName ?? '',
        uri: `/accounts/${accountName}`,
        noTranslate: true,
      },
    ]);
  }, [accountName]);

  if (error) return <div>failed to load</div>;
  if (!account) return <div>{error}</div>;
  return (
    <div>
      <div className="flex items-center gap-4 pb-6">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="flex-1 shrink-0 whitespace-nowrap text-xl font-semibold tracking-tight sm:grow-0">{account.alias ?? account.name}</h1>
            <Badge variant="outline" className="ml-auto sm:ml-0">
              {account.status}
            </Badge>
          </div>
          {!!account.alias && <h4 className="text-sm text-gray-500">{account.name}</h4>}
        </div>

        <div className="hidden items-center gap-2 md:ml-auto md:flex">
          <div className="text-right">
            <div className="flex items-center justify-end gap-1 text-lg">
              {Object.keys(account.amount.detail).length > 1 && <p>≈</p>}
              <Amount amount={account.amount.calculated.number} currency={account.amount.calculated.currency}></Amount>
            </div>
            {Object.keys(account.amount.detail).length > 1 && (
              <div className="flex flex-col gap-0.5 items-end text-base">
                {Object.entries(account.amount.detail ?? {}).map(([key, value]) => (
                  <Amount key={key} amount={value} currency={key}></Amount>
                ))}
              </div>
            )}
          </div>
        </div>
      </div>

      <div className="my-4">
        {account_balance_error ? (
          <div>fail to fetch account balance history</div>
        ) : (
          account_balance_data && <AccountBalanceHistoryGraph data={account_balance_data.balance} />
        )}
      </div>

      <Tabs defaultValue="journals">
        <TabsList>
          <TabsTrigger value="journals">
            <Notebook className="w-4 h-4 mr-2" /> Journals
          </TabsTrigger>
          <TabsTrigger value="documents">
            <FileStack className="w-4 h-4 mr-2" /> Documents
          </TabsTrigger>
          <TabsTrigger value="settings">
            <Cog className="w-4 h-4 mr-2" /> Settings
          </TabsTrigger>
        </TabsList>

        <TabsContent value="journals">
          <Card className="mt-2 rounded-sm ">
            <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2 ">
              <CardTitle>Account Journals</CardTitle>
            </CardHeader>
            <CardContent>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Date</TableHead>
                    <TableHead>Payee & Narration</TableHead>
                    <TableHead className="text-right ">Change Amount</TableHead>
                    <TableHead className="text-right ">After Change Amount</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  <LoadingComponent
                    fetcherFunction={retrieveAccountJournals}
                    params={{ account_name: accountName ?? '' }}
                    skeleton={<div>loading</div>}
                    render={(data) => (
                      <>
                        {(data.data ?? []).map((item) => (
                          <TableRow>
                            <TableCell>{format(new Date(item.datetime), 'yyyy-MM-dd HH:mm:ss')}</TableCell>
                            <TableCell>
                              <PayeeNarration payee={item.payee} narration={item.narration} />
                            </TableCell>
                            <TableCell className="text-right ">
                              <Amount amount={item.inferred_unit_number} currency={item.inferred_unit_commodity} />
                            </TableCell>
                            <TableCell className="text-right ">
                              <Amount amount={item.account_after_number} currency={item.account_after_commodity} />
                            </TableCell>
                          </TableRow>
                        ))}
                      </>
                    )}
                  />
                </TableBody>
              </Table>
            </CardContent>
          </Card>
        </TabsContent>
        <TabsContent value="documents">
          <Card className="mt-2 rounded-sm ">
            <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2 ">
              <CardTitle>Account Documents</CardTitle>
            </CardHeader>
            <CardContent>
              <LoadingComponent
                fetcherFunction={retrieveAccountDocuments}
                params={{ account_name: accountName ?? '' }}
                skeleton={<div>loading</div>}
                render={(data) => (
                  <>
                    <ImageLightBox src={lightboxSrc} onChange={setLightboxSrc} />
                    <div className="grid grid-cols-1 xs:grid-cols-1 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-2 sm:gap-3 md:gap-4">
                      <AccountDocumentUpload url={`/api/accounts/${accountName}/documents`} />
                      {(data.data ?? []).map((document, idx) => (
                        <DocumentPreview onClick={(path) => setLightboxSrc(path)} key={idx} uri={document.path} filename={document.path} />
                      ))}
                    </div>
                  </>
                )}
              ></LoadingComponent>
            </CardContent>
          </Card>
        </TabsContent>
        <TabsContent value="settings">
          <Card className="mt-2 rounded-sm ">
            <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2 ">
              <CardTitle>Settings</CardTitle>
            </CardHeader>
            <CardContent>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Currency</TableHead>
                    <TableHead>Current Balance</TableHead>
                    <TableHead>Latest Balance Time</TableHead>
                    <TableHead>Pad Account</TableHead>
                    <TableHead>Destination</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {Object.entries(account?.amount.detail ?? {}).map(([commodity, amount]) => (
                    <AccountBalanceCheckLine key={commodity} currentAmount={amount} commodity={commodity} accountName={account.name} />
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

export default SingleAccount;
