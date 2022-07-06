import { Flex, Text } from "@chakra-ui/react";
interface Props {
    amount?: string,
    currency?: string,
    negetive?: boolean
}
export default function Amount({ amount, currency, negetive }: Props) {
    const flag = negetive || false ? -1 : 1;
    var formatter = new Intl.NumberFormat('en-US', {
        minimumFractionDigits: 2,
        maximumFractionDigits: 10,
    });
    const parsedValue = parseFloat(amount || "0");
    const value = parsedValue === 0 ? parsedValue : flag * parsedValue;
    return (
        <Flex display={"inline-flex"}>
            <Text>{formatter.format(value)}</Text>
            <Text mx={1}>{currency}</Text>
        </Flex>
    )
}