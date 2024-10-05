import { useDisclosure, useMediaQuery } from '@mantine/hooks';
import { useState } from 'react';

import { IconSquarePlus } from '@tabler/icons-react';
import { axiosInstance } from '../global.ts';
import { showNotification } from '@mantine/notifications';
import TransactionEditForm from './TransactionEditForm';
import { Button } from './ui/button.tsx';
import { useTranslation } from 'react-i18next';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger } from './ui/dialog.tsx';

export default function NewTransactionButton() {
  const isMobile = useMediaQuery('(max-width: 600px)');
  const { t } = useTranslation();
  const [isOpen, isOpenHandler] = useDisclosure(false);
  const [data, setData] = useState<any>({});
  const [isValid, setIsValid] = useState<boolean>(false);

  const onCreate = () => {
    axiosInstance
      .post(`/api/transactions`, data)
      .then((res) => {
        isOpenHandler.close();
        showNotification({
          title: 'New transaction is created',
          message: '',
        });
      })
      .catch(function (error) {
        showNotification({
          title: 'Fail to create new Transaction',
          color: 'red',
          message: error?.response?.data ?? '',
          autoClose: false,
        });
        console.log(error);
      });
  };

  return (
    <>
      <Dialog open={isOpen} onOpenChange={isOpenHandler.toggle} >
        <DialogTrigger>
          <Button onClick={() => isOpenHandler.open()}>
          {t('NEW_TRANSACTION_BUTTON')}
        </Button>
        </DialogTrigger>
        <DialogContent className="sm:max-w-[600px]">
          <DialogHeader>
            <DialogTitle>{t('NEW_TRANSACTION_DIALOG_TITLE')}</DialogTitle>
            <DialogDescription hidden>
              {t('NEW_TRANSACTION_DIALOG_DESCRIPTION')}
            </DialogDescription>
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
