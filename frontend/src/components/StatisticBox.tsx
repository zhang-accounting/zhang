import { Flex, Text } from "@chakra-ui/react";
import { useTranslation } from "react-i18next";
import Amount from "./Amount";
interface Props {
    text: string,
    amount: string,
    currency: string,
    detail?: any,
    negetive?: boolean
}
export default function StatisticBox({ text, amount, currency, negetive }: Props) {
    const {t} = useTranslation();
    
    const displayBox = <Flex direction={"column"} mx={3}>

        <Text fontSize={"x-small"} color={"gray.700"}>{t(text)}</Text>
        <Amount amount={amount} negetive={negetive} currency={currency} />
    </Flex>;
    return displayBox;

}