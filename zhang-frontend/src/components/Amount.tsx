import { Flex,Text } from "@chakra-ui/react";

export default function Component({amount, currency}) {
    return (
        <Flex>
            <Text>{amount}</Text>
            <Text mx={1}>{currency}</Text>
        </Flex>
    )
}