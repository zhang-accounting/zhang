import { Flex, Text, Tooltip } from "@chakra-ui/react";
import Amount from "./Amount";
interface Props {
    text: string,
    amount: string,
    currency: string,
    detail?: any,
    negetive?: boolean
}
export default function Component({ text, amount, currency, detail, negetive }: Props) {
    const negative = (negetive || false) ? -1 : 1;
    let detailTip = (
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

    var formatter = new Intl.NumberFormat('en-US', {
    });

    const displayBox = <Flex direction={"column"} mx={3}>

        <Text fontSize={"x-small"} color={"gray.700"}>{text}</Text>
        <Amount amount={formatter.format(parseFloat(amount) * negative)} currency={currency} />
    </Flex>;
    return detail === undefined ? displayBox : (<Tooltip hasArrow label={detailTip} shouldWrapChildren>
        {displayBox}
    </Tooltip>);



}