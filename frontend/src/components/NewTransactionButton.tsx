import { useSetAtom } from 'jotai';
import { Plus } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { toast } from 'sonner';
import { createNewTransaction } from '@/api/requests';
import { apiErrorMessage } from '@/lib/api-error';
import { ledgerChangedAtom } from '@/states/ledger';
import TransactionEditForm, { TransactionFormValue } from './TransactionEditForm';
import { AutoDrawer, AutoDrawerTrigger } from './ui/auto-drawer';
import { Button } from './ui/button';
import { Spinner } from './ui/spinner';
import { cn } from '@/lib/utils';

const TRIGGER_CLASS = {
  /** Mobile top bar: 40px turquoise square with a plus. */
  icon: 'size-10 px-0',
  /**
   * Desktop sidebar: full-width card button (36px) with a link-coloured plus; a 32px icon button when the sidebar is
   * collapsed to icons.
   */
  sidebar: cn(
    'h-9 w-full justify-start gap-2 bg-card px-2.5 font-medium text-foreground shadow-xs hover:bg-card hover:text-foreground dark:bg-card dark:hover:bg-accent',
    '[&_svg]:text-link group-data-[collapsible=icon]:size-8 group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:px-0',
  ),
};

/**
 * "New transaction" action (sidebar button on desktop, "+" in the mobile top bar): Dialog >= md, bottom Drawer < md. The
 * form resets every time it opens.
 */
export default function NewTransactionButton({ variant = 'icon' }: { variant?: keyof typeof TRIGGER_CLASS }) {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(false);
  const [formKey, setFormKey] = useState(0);
  const [data, setData] = useState<TransactionFormValue | undefined>(undefined);
  const [isValid, setIsValid] = useState<boolean>(false);
  const [saving, setSaving] = useState(false);
  const ledgerChanged = useSetAtom(ledgerChangedAtom);

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
      ledgerChanged();
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
      <AutoDrawerTrigger
        render={
          <Button
            variant={variant === 'sidebar' ? 'outline' : 'default'}
            className={TRIGGER_CLASS[variant]}
            aria-label={t('NEW_TRANSACTION_BUTTON')}
            title={variant === 'sidebar' ? t('NEW_TRANSACTION_BUTTON') : undefined}
          />
        }
      >
        <Plus />
        {variant === 'sidebar' && <span className="truncate group-data-[collapsible=icon]:hidden">{t('NEW_TRANSACTION_BUTTON')}</span>}
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
