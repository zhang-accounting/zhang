import { Flex, Text } from "@chakra-ui/react";
interface Props {
    amount: string,
    currency: string
}
export default function Component({ amount, currency }: Props) {
    return (
        <Flex>
            <Text>{amount}</Text>
            <Text mx={1}>{currency}</Text>
        </Flex>
    )
}