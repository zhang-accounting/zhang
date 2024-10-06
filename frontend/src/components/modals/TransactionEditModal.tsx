import TransactionEditForm from '../TransactionEditForm';
import { useEffect, useState } from 'react';
import { axiosInstance } from '../../global.ts';
import { useAtom } from 'jotai';
import { editTransactionAtom } from '../../states/journals';
import { useDisclosure } from '@mantine/hooks';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../ui/dialog.tsx';

import { t } from 'i18next';
import { Button } from '../ui/button.tsx';
import { toast } from 'sonner';
export const TransactionEditModal = () => {
  const [isOpen, isOpenHandler] = useDisclosure(false);
  const [editTransaction, setEditTransaction] = useAtom(editTransactionAtom);
  const [data, setData] = useState<any>({});
  const [isValid, setIsValid] = useState<boolean>(false);
  const onUpdate = () => {
    axiosInstance
      .put(`/api/transactions/${editTransaction?.id}`, data)
      .then(() => {
        toast.success('transaction is updated');
        setEditTransaction(undefined);
        isOpenHandler.close();
      })
      .catch(function (error) {
        toast.error('Fail to update new Transaction', {
          description: error?.response?.data ?? '',
        });
        console.error(error);
      });
  };
  const onChange = (open: boolean) => {
    if (open === false) {
      setEditTransaction(undefined);
    }
    isOpenHandler.toggle();
  };

  useEffect(() => {
    if (editTransaction) {
      setData(editTransaction);
      isOpenHandler.open();
    }
  }, [editTransaction]);

  return (
    <>
      <Dialog open={isOpen} onOpenChange={onChange}>
        <DialogContent className="sm:max-w-[660px]">
          <DialogHeader>
            <DialogTitle>{t('TRANSACTION_EDIT_MODAL_TITLE')}</DialogTitle>
            <DialogDescription hidden>{t('TRANSACTION_EDIT_MODAL_DESCRIPTION')}</DialogDescription>
          </DialogHeader>
          <TransactionEditForm
            data={editTransaction}
            onChange={(data, isValid) => {
              setData(data);
              setIsValid(isValid);
            }}
          ></TransactionEditForm>

          <DialogFooter>
            <Button variant="outline" onClick={() => onChange(false)}>
              {t('TRANSACTION_EDIT_MODAL_CLOSE')}
            </Button>
            <Button onClick={onUpdate} disabled={!isValid}>
              {t('TRANSACTION_EDIT_MODAL_SAVE')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
};
