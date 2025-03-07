import { useDisclosure } from '@mantine/hooks';
import { useState } from 'react';

import { createNewTransaction } from '@/api/requests.ts';
import { useTranslation } from 'react-i18next';
import { toast } from 'sonner';
import TransactionEditForm from './TransactionEditForm';
import { Button } from './ui/button.tsx';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger } from './ui/dialog.tsx';

export default function NewTransactionButton() {
  const { t } = useTranslation();
  const [isOpen, isOpenHandler] = useDisclosure(false);
  const [data, setData] = useState<any>({});
  const [isValid, setIsValid] = useState<boolean>(false);

  const onCreate = async () => {
    try {
      await createNewTransaction(data);
      isOpenHandler.close();
      toast.success('New transaction is created');
    } catch (error) {
      toast.error('Fail to create new Transaction', {
        description: String(error),
      });
    }
  };

  return (
    <>
      <Dialog open={isOpen} onOpenChange={isOpenHandler.toggle}>
        <DialogTrigger>
          <Button onClick={() => isOpenHandler.open()}>{t('NEW_TRANSACTION_BUTTON')}</Button>
        </DialogTrigger>
        <DialogContent className="sm:max-w-[600px]">
          <DialogHeader>
            <DialogTitle>{t('NEW_TRANSACTION_DIALOG_TITLE')}</DialogTitle>
            <DialogDescription hidden>{t('NEW_TRANSACTION_DIALOG_DESCRIPTION')}</DialogDescription>
          </DialogHeader>
          <TransactionEditForm
            onChange={(data, isValid) => {
              setData(data);
              console.log(isValid);
              setIsValid(isValid);
            }}
          ></TransactionEditForm>

          <DialogFooter>
            <Button variant="outline" onClick={isOpenHandler.close}>
              {t('NEW_TRANSACTION_CANCEL')}
            </Button>
            <Button onClick={onCreate} disabled={!isValid}>
              {t('NEW_TRANSACTION_SAVE')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
