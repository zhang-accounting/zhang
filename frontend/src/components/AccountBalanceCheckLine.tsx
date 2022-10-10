import { gql, useMutation } from '@apollo/client';
import { TextInput, Button } from '@mantine/core';
import { format } from 'date-fns';
import { useState } from 'react';
interface Props {
  currency: string;
  accountName: string;
}
export default function AccountBalanceCheckLine({ currency, accountName }: Props) {
  const [amount, setAmount] = useState('');

  const [appendData] = useMutation(
    gql`
      mutation APPEND_DATA($date: Int, $content: String) {
        appendData(date: $date, content: $content)
      }
    `,
    {
      refetchQueries: ['FILE_LIST', 'SINGLE_FILE_ENTRY', 'JOURNAL_LIST', 'BAR_STATISTIC'],
    },
  );

  const submitCheck = () => {
    const date = new Date();
    const dateDisplay = format(date, 'yyyy-MM-dd hh:mm:ss');
    const content = `${dateDisplay} balance ${accountName} ${amount} ${currency}`;
    appendData({
      variables: {
        date: Math.round(date.getTime() / 1000),
        content: `\n${content}\n`,
      },
    });
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
        }></TextInput>
    </>
  );
}
