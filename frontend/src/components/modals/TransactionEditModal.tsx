import { ContextModalProps } from '@mantine/modals';
import { JournalTransactionItem } from '../../rest-model';
import TransactionEditForm from '../TransactionEditForm';
import { useState } from 'react';
import { Button, Group } from '@mantine/core';
import { axiosInstance } from '../../index';
import { showNotification } from '@mantine/notifications';

export const TransactionEditModal = ({
                                       context,
                                       id,
                                       innerProps,
                                     }: ContextModalProps<{
  data: JournalTransactionItem;
}>) => {
  const [data, setData] = useState<any>({});
  const [isValid, setIsValid] = useState<boolean>(false);
  const onUpdate = () => {
    axiosInstance
      .put(`/api/transactions/${innerProps.data.id}`, data)
      .then((res) => {
        showNotification({
          title: 'transaction is updated',
          message: '',
        });
        context.closeModal('transactionEditModal');
      })
      .catch(function(error) {
        showNotification({
          title: 'Fail to update new Transaction',
          color: 'red',
          message: error?.response?.data ?? '',
          autoClose: false,
        });
        console.log(error);
      });
  };
  return (
    <>
      <TransactionEditForm
        data={innerProps.data}
        onChange={(data, isValid) => {
          setData(data);
          setIsValid(isValid);
        }}
      ></TransactionEditForm>

      <Group justify="right" my="md">
        <Button mr={3} onClick={onUpdate} disabled={!isValid}>
          Save
        </Button>
      </Group>
    </>
  );
};
