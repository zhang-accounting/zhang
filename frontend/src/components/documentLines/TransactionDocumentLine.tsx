import { Badge, Box, Flex, Text, Image, useColorModeValue } from "@chakra-ui/react";
import { TransactionDto } from "../../gql/jouralList";


export interface Props {
    date: number,
    filename: string,
    transaction: TransactionDto
}


export default function TransactionDocumentDto({ filename, transaction }: Props) {
    const extension = filename.split(".").pop()?.toUpperCase();
    const simpleFilename = filename.split("/").pop() || "";
    return (
        <Flex flexDirection={"column"} flex="0 0 20%" m={4} p={4} border="1px"
            borderColor={useColorModeValue('gray.200', 'gray.700')}>
            <Box>
                <Image src="https://images.unsplash.com/photo-1511216335778-7cb8f49fa7a3?ixid=MnwxMjA3fDB8MHxwaG90by1wYWdlfHx8fGVufDB8fHx8&ixlib=rb-1.2.1&auto=format&fit=crop&w=720&q=80" />
            </Box>
            <Flex flexDirection={"column"} mt={2}>
                <Flex alignItems={"center"}>
                    <Badge variant='outline'>{extension}</Badge>
                    <Text mx={2}>{simpleFilename}</Text>
                </Flex>
                <Box>
                    <Text mx={1} fontWeight={"bold"}>{transaction.payee}</Text>
                    <Text mx={1}>{transaction.narration}</Text>
                </Box>
            </Flex>

        </Flex>
    )
}