import { useDisclosure, useMediaQuery } from '@mantine/hooks';
import { useState } from 'react';

import { Button, Container, Group, Modal } from '@mantine/core';
import { IconSquarePlus } from '@tabler/icons-react';
import { axiosInstance } from '..';
import { showNotification } from '@mantine/notifications';
import TransactionEditForm from './TransactionEditForm';

export default function NewTransactionButton() {
  const isMobile = useMediaQuery('(max-width: 600px)');

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
      .catch(function(error) {
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
      <Button size="xs" fullWidth leftSection={<IconSquarePlus />} onClick={() => isOpenHandler.open()}>
        NEW TRANSACTION
      </Button>

      <Modal
        onClose={() => isOpenHandler.close()}
        opened={isOpen}
        size="xl"
        centered
        closeOnEscape
        // overflow="inside"
        title="New Transaction"
        fullScreen={isMobile}
      >
        <Container>
          <TransactionEditForm
            onChange={(data, isValid) => {
              setData(data);
              console.log(isValid);
              setIsValid(isValid);
            }}
          ></TransactionEditForm>

          <Group justify="right" my="md">
            <Button variant="outline" onClick={isOpenHandler.close}>
              Cancel
            </Button>
            <Button mr={3} onClick={onCreate} disabled={!isValid}>
              Save
            </Button>
          </Group>
        </Container>
      </Modal>
    </>
  );
}
