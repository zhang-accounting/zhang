import { Flex, Text, Box } from "@chakra-ui/react";
import { useNavigate } from "react-router";
import AccountTrie from "../utils/AccountTrie";
import Amount from "./Amount";

interface Props {
    data: AccountTrie
}

export default function AccountLine({ data }: Props) {
    let navigate = useNavigate();

    const onNavigate = () => {
        if (data?.val?.name) {
            navigate(data?.val?.name)
        }
    }

    return (
        <Flex direction={"column"} py={1} pl={2} border={"1px"} borderColor={"gray.200"} >
            <Flex py={1} justifyContent={"space-between"} _hover={{ backgroundColor: "gray.200" }}>
            <Text onClick={onNavigate} cursor={data.isNode ? "pointer" : "default"}>{data.word}</Text>
                {data.isNode && (
                    <Amount amount={data!.val!.snapshot.summary.number} currency={data!.val!.snapshot.summary.currency}></Amount>
                )}
            </Flex>
            <Box pl={2}>
                {Object.keys(data.children).sort().map((child, idx) => (
                    <AccountLine key={idx} data={data.children[child]} />
                ))}
            </Box>
        </Flex>
    )
}