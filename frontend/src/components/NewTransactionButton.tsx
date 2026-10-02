import { useSetAtom } from 'jotai';
import { Plus } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { toast } from 'sonner';
import { createNewTransaction } from '@/api/requests';
import { apiErrorMessage } from '@/lib/api-error';
import { accountFetcher } from '@/states/account';
import { journalFetcher } from '@/states/journals';
import TransactionEditForm, { TransactionFormValue } from './TransactionEditForm';
import { AutoDrawer, AutoDrawerTrigger } from './ui/auto-drawer';
import { Button } from './ui/button';
import { Spinner } from './ui/spinner';

/** Top-bar "New transaction" action: Dialog >= md, bottom Drawer < md. The form resets every time it opens. */
export default function NewTransactionButton() {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(false);
  const [formKey, setFormKey] = useState(0);
  const [data, setData] = useState<TransactionFormValue | undefined>(undefined);
  const [isValid, setIsValid] = useState<boolean>(false);
  const [saving, setSaving] = useState(false);
  const refreshJournals = useSetAtom(journalFetcher);
  const refreshAccounts = useSetAtom(accountFetcher);

  const onOpenChange = (open: boolean) => {
    if (open) setFormKey((key) => key + 1);
    setIsOpen(open);
  };

  const onCreate = async () => {
    if (!data) return;
    setSaving(true);
    try {
      await createNewTransaction(data);
      setIsOpen(false);
      toast.success(t('ledger.txn.created'));
      refreshJournals();
      refreshAccounts();
    } catch (error) {
      toast.error(t('ledger.txn.create_failed'), { description: await apiErrorMessage(error) });
    } finally {
      setSaving(false);
    }
  };

  return (
    <AutoDrawer
      open={isOpen}
      onOpenChange={onOpenChange}
      className="sm:max-w-2xl md:max-h-[90vh] md:overflow-y-auto"
      title={t('NEW_TRANSACTION_DIALOG_TITLE')}
      description={t('NEW_TRANSACTION_DIALOG_DESCRIPTION')}
      footer={
        <>
          <Button variant="outline" className="h-10 md:h-8" onClick={() => setIsOpen(false)}>
            {t('NEW_TRANSACTION_CANCEL')}
          </Button>
          <Button className="h-10 md:h-8" onClick={onCreate} disabled={!isValid || saving}>
            {saving && <Spinner data-icon="inline-start" />}
            {t('NEW_TRANSACTION_SAVE')}
          </Button>
        </>
      }
    >
      <AutoDrawerTrigger render={<Button className="size-10 px-0 sm:h-8 sm:w-auto sm:px-2.5" aria-label={t('NEW_TRANSACTION_BUTTON')} />}>
        <Plus />
        <span className="hidden sm:inline">{t('NEW_TRANSACTION_BUTTON')}</span>
      </AutoDrawerTrigger>
      <TransactionEditForm
        key={formKey}
        onChange={(data, isValid) => {
          setData(data);
          setIsValid(isValid);
        }}
      />
    </AutoDrawer>
  );
}
