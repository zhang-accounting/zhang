import { gql, useMutation } from "@apollo/client";
import { Button, Input, InputGroup, InputRightElement } from "@chakra-ui/react";
import { format } from "date-fns";
import { useState } from "react";
interface Props {
    currency: string,
    accountName: string
}
export default function Component({ currency, accountName }: Props) {
    const [amount, setAmount] = useState("")

    const [appendData, _] = useMutation(gql`
    mutation APPEND_DATA($date: Int, $content: String) {
        appendData(date: $date, content: $content) 
    }
    `, {
        refetchQueries: ["FILE_LIST", "SINGLE_FILE_ENTRY", "JOURNAL_LIST", "BAR_STATISTIC"]
    })

    const submitCheck = () => {
        const date = new Date();
        const dateDisplay = format(date, "yyyy-MM-dd hh:mm:ss");
        const content = `${dateDisplay} balance ${accountName} ${amount} ${currency}`;
        appendData({
            variables: { date: Math.round(date.getTime() / 1000), content: `\n${content}\n` }
        });
        setAmount("")
    }
    return (
        <InputGroup size='md'>
            <Input
                pr='4.5rem'
                type='number'
                placeholder={` Balanced ${currency} Amount`}
                value={amount}
                onChange={e => setAmount(e.target.value)}
            />
            <InputRightElement width='4.5rem' px={2}>
                <Button h='1.75rem' size='sm' onClick={submitCheck} disabled={amount.length === 0}>
                    Check
                </Button>
            </InputRightElement>
        </InputGroup>
    )
}