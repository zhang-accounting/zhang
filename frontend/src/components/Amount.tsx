import { Flex, Text } from "@chakra-ui/react";
interface Props {
    amount?: string,
    currency?: string,
    negetive?: boolean
}
export default function Amount({ amount, currency, negetive }: Props) {
    const flag = negetive || false ? -1 : 1;

    return (
        <Flex>
            <Text>{flag * parseFloat(amount || "0")}</Text>
            <Text mx={1}>{currency}</Text>
        </Flex>
    )
}