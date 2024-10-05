import TransactionEditForm from '../TransactionEditForm';
import { useEffect, useState } from 'react';
import { showNotification } from '@mantine/notifications';
import { axiosInstance } from '../../global.ts';
import { useAtom } from 'jotai';
import { editTransactionAtom } from '../../states/journals';
import { useDisclosure } from '@mantine/hooks';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../ui/dialog.tsx';

import { t } from 'i18next';
import { Button } from '../ui/button.tsx';
export const TransactionEditModal = () => {

  const [isOpen, isOpenHandler] = useDisclosure(false);
  const [editTransaction, setEditTransaction] = useAtom(editTransactionAtom);
  const [data, setData] = useState<any>({});
  const [isValid, setIsValid] = useState<boolean>(false);
  const onUpdate = () => {
    axiosInstance
      .put(`/api/transactions/${editTransaction?.id}`, data)
      .then((res) => {
        showNotification({
          title: 'transaction is updated',
          message: '',
        });
        setEditTransaction(undefined);
        isOpenHandler.close();
      })
      .catch(function (error) {
        showNotification({
          title: 'Fail to update new Transaction',
          color: 'red',
          message: error?.response?.data ?? '',
          autoClose: false,
        });
        console.error(error);
      });
  };
  const onChange = (open: boolean) => {
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

      <Dialog open={isOpen} onOpenChange={onChange}  >
        <DialogContent className="sm:max-w-[600px]">
          <DialogHeader>
            <DialogTitle>{t('TRANSACTION_EDIT_MODAL_TITLE')}</DialogTitle>
            <DialogDescription hidden>
              {t('TRANSACTION_EDIT_MODAL_DESCRIPTION')}
            </DialogDescription>
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
