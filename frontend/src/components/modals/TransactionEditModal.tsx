import { useAtom, useSetAtom } from 'jotai';
import { Info } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation } from 'react-router';
import { toast } from 'sonner';
import { updateTransaction } from '@/api/requests';
import { JournalTransactionItem } from '@/api/types';
import { apiErrorMessage } from '@/lib/api-error';
import { accountFetcher } from '../../states/account';
import { editTransactionAtom, journalFetcher } from '../../states/journals';
import { transactionEditBlocker } from '../journalLines/journal-utils';
import TransactionEditForm, { TransactionFormValue } from '../TransactionEditForm';
import { AutoDrawer } from '../ui/auto-drawer';
import { Button } from '../ui/button';
import { Spinner } from '../ui/spinner';
import { RawEditNote } from './RawEditNote';

/**
 * Edit an existing transaction: Dialog >= md, bottom Drawer < md. Opens whenever `editTransactionAtom` is set.
 *
 * The update API rewrites the whole transaction from the form (account, `number commodity` and metadata per posting). Cost / price are
 * detected from the payload and block editing (Raw Edit instead); posting comments and posting flags are not in the payload,
 * so saving asks for an explicit confirmation that they will be dropped.
 */
export const TransactionEditModal = () => {
  const { t } = useTranslation();
  const { pathname } = useLocation();
  const [editTransaction, setEditTransaction] = useAtom(editTransactionAtom);
  const refreshJournals = useSetAtom(journalFetcher);
  const refreshAccounts = useSetAtom(accountFetcher);
  const [shown, setShown] = useState<JournalTransactionItem | undefined>(editTransaction);
  const [data, setData] = useState<TransactionFormValue | undefined>(undefined);
  const [isValid, setIsValid] = useState<boolean>(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (editTransaction) setShown(editTransaction);
  }, [editTransaction]);

  // The atom is global: never carry an open editor over to another page.
  useEffect(() => () => setEditTransaction(undefined), [pathname, setEditTransaction]);

  const blocked = shown ? transactionEditBlocker(shown) !== null : false;

  const onOpenChange = (open: boolean) => {
    if (!open) setEditTransaction(undefined);
  };

  const onUpdate = async () => {
    if (!shown || !data || blocked) return;
    if (!window.confirm(t('ledger.txn.edit_confirm_rewrite'))) return;
    setSaving(true);
    try {
      await updateTransaction({ ...data, transaction_id: shown.id });
      toast.success(t('ledger.txn.updated'));
      setEditTransaction(undefined);
      refreshJournals();
      refreshAccounts();
    } catch (error) {
      toast.error(t('ledger.txn.update_failed'), { description: await apiErrorMessage(error) });
    } finally {
      setSaving(false);
    }
  };

  return (
    <AutoDrawer
      open={editTransaction !== undefined}
      onOpenChange={onOpenChange}
      className="sm:max-w-2xl md:max-h-[90vh] md:overflow-y-auto"
      title={t('TRANSACTION_EDIT_MODAL_TITLE')}
      description={t('ledger.txn.edit_description')}
      footer={
        <>
          <Button variant="outline" className="h-10 md:h-8" onClick={() => onOpenChange(false)}>
            {t('TRANSACTION_EDIT_MODAL_CLOSE')}
          </Button>
          <Button className="h-10 md:h-8" onClick={onUpdate} disabled={!isValid || saving || blocked}>
            {saving && <Spinner data-icon="inline-start" />}
            {t('TRANSACTION_EDIT_MODAL_SAVE')}
          </Button>
        </>
      }
    >
      {shown && blocked && <RawEditNote className="mt-4 md:mt-0" />}
      {shown && !blocked && (
        <p className="mt-4 flex gap-2 text-xs text-muted-foreground md:mt-0">
          <Info className="mt-px size-3.5 shrink-0" aria-hidden />
          {t('ledger.txn.edit_rewrite_note')}
        </p>
      )}
      {shown && !blocked && (
        <TransactionEditForm
          key={shown.id}
          data={shown}
          onChange={(data, isValid) => {
            setData(data);
            setIsValid(isValid);
          }}
        />
      )}
    </AutoDrawer>
  );
};
