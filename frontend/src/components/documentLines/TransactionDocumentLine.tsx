import { Badge, Box, Flex, Text } from "@chakra-ui/react";
import { TransactionDto } from "../../gql/jouralList";

export interface Props {
    date: number,
  filename: string,
  transaction: TransactionDto
  }
  

export default function TransactionDocumentDto({ filename, transaction }: Props) {
    const extension = filename.split(".").pop()?.toUpperCase();

    return (
        <Flex m={2} justifyContent="space-between">
            <Flex alignItems={"center"} alignContent="center">
                <Badge variant='outline'>{extension}</Badge>
                <Text mx={2}>{filename}</Text>
            </Flex>
            <Box m={1}>
                <Text mx={1} fontWeight={"bold"}>{transaction.payee}</Text>
                <Text mx={1}>{transaction.narration}</Text>
            </Box>
            
        </Flex>
    )
}