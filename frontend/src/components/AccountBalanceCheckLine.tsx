import {TextInput, Button} from '@mantine/core';
import {useState} from 'react';
import {axiosInstance} from "../index";
import {showNotification} from "@mantine/notifications";
import {useAppDispatch} from "../states";
import {accountsSlice} from "../states/account";

interface Props {
  currency: string;
  accountName: string;
}

export default function AccountBalanceCheckLine({currency, accountName}: Props) {
  const [amount, setAmount] = useState('');
  const dispatch = useAppDispatch();

  const onSave = async () => {
    try {
      await axiosInstance.post(`/api/accounts/${accountName}/balances`, {
        type: "Balance",
        account_name: accountName,
        amount: {
          number: amount,
          commodity: currency,
        }
      });
      showNotification({
        title: 'Balance account successfully',
        message: ""
      });
      dispatch(accountsSlice.actions.clear());
    } catch (e: any) {
      showNotification({
        title: 'Fail to Balance Account',
        color: 'red',
        message: e?.response?.data ?? "",
        autoClose: false
      });
    }


  };

  const submitCheck = () => {
    onSave();
    setAmount('');
  };
  return (
      <>
        <TextInput
            placeholder={`Balanced ${currency} Amount`}
            value={amount}
            onChange={(e) => setAmount(e.target.value)}
            rightSectionWidth={75}
            rightSection={
              <Button size="xs" onClick={submitCheck} disabled={amount.length === 0}>
                Check
              </Button>
            }
        ></TextInput>
      </>
  );
}
