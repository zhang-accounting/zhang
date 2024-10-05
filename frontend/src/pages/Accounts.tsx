import { useDocumentTitle, useInputState, useLocalStorage } from '@mantine/hooks';
import AccountLine from '../components/AccountLine';
import { AccountStatus } from '../rest-model';
import { loadable_unwrap } from '../states';
import { accountAtom, accountFetcher } from '../states/account';
import { useTranslation } from 'react-i18next';
import { useAtom, useAtomValue, useSetAtom } from 'jotai';
import { selectAtom } from 'jotai/utils';
import AccountTrie from '../utils/AccountTrie';
import { titleAtom } from '../states/basic';
import { useMemo } from 'react';
import { Table, TableBody, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { Input } from '@/components/ui/input';
import { Button } from '@/components/ui/button';
import { Switch } from '@/components/ui/switch';
import { Label } from '@/components/ui/label';

export default function Accounts() {
  const { t } = useTranslation();
  const [filterKeyword, setFilterKeyword] = useInputState('');
  const [hideClosedAccount, setHideClosedAccount] = useLocalStorage({ key: 'hideClosedAccount', defaultValue: false });

  const [accountTrie] = useAtom(
    useMemo(
      () =>
        selectAtom(accountAtom, (val) => {
          return loadable_unwrap(val, new AccountTrie(), (data) => {
            let trie = new AccountTrie();
            for (let account of data.filter((it) => (hideClosedAccount ? it.status === AccountStatus.Open : true))) {
              let trimmedKeyword = filterKeyword.trim();
              if (trimmedKeyword !== '') {
                if (
                  account.name.toLowerCase().includes(trimmedKeyword.toLowerCase()) ||
                  (account.alias?.toLowerCase() ?? '').includes(trimmedKeyword.toLowerCase())
                ) {
                  trie.insert(account);
                }
              } else {
                trie.insert(account);
              }
            }
            return trie;
          });
        }),
      [filterKeyword, hideClosedAccount],
    ),
  );

  const refreshAccounts = useSetAtom(accountFetcher);

  const ledgerTitle = useAtomValue(titleAtom);

  useDocumentTitle(`Accounts - ${ledgerTitle}`);

  return (
    <div>

<div className="flex flex-1 items-center justify-between space-x-2 mb-4">
        <div className="flex flex-1 space-x-2 items-center">
        <Input
          className="w-[33%]"
          placeholder={t('ACCOUNT_FILTER_PLACEHOLDER')}
          value={filterKeyword}
          onChange={setFilterKeyword}
        />

<div className="flex items-center space-x-2">
      <Switch id="airplane-mode" checked={hideClosedAccount} onCheckedChange={setHideClosedAccount} />
      <Label htmlFor="airplane-mode" className={hideClosedAccount ? '' : 'text-gray-500'}>Hide closed accounts</Label>
    </div>
        </div>
        <Button
          variant="outline"
          onClick={() => refreshAccounts()}
        >
          {t('REFRESH')}
        </Button>
      </div>
      
      

      <div className="rounded-md border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Name</TableHead>
            <TableHead className="text-right">Balance</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {Object.keys(accountTrie.children)
            .sort()
            .map((item) => (
              <AccountLine spacing={0} key={accountTrie.children[item].path} data={accountTrie.children[item]} />
            ))}
        </TableBody>
      </Table>
      </div>
    </div>
  );
}
