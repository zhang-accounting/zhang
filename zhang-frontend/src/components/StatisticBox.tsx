import { Flex, Text, Tooltip } from "@chakra-ui/react";
import Amount from "./Amount";

export default function Component({ text, amount, currency }) {
    let detail = (
        <Flex direction={"column"}>
            <Flex justifyContent={"space-between"}>
                <Text>1000</Text>
                <Text ml={3}>CNY</Text>
            </Flex>
            <Flex justifyContent={"space-between"}>
                <Text>1000</Text>
                <Text>CNY</Text>
            </Flex>
            <Flex justifyContent={"space-between"}>
                <Text>1000</Text>
                <Text>CNY</Text>
            </Flex>
        </Flex>
    )
    return (
        <Tooltip hasArrow label={detail} shouldWrapChildren>

            <Flex direction={"column"} mx={2}>

                <Text fontSize={"x-small"} color={"gray.700"}>{text}</Text>
                <Amount amount={amount} currency={currency} />

            </Flex>
        </Tooltip>
    )
}