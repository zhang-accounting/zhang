import { Badge, Box, Flex, Text, Image, useColorModeValue } from "@chakra-ui/react";
import { TransactionDto } from "../../gql/jouralList";
import { EXTENSIONS_SUPPORT_PREVIEW } from "./AccountDocumentLine";
import { Buffer } from "buffer";

export interface Props {
    date: number,
    filename: string,
    transaction: TransactionDto
}


export default function TransactionDocumentDto({ filename, transaction }: Props) {
    const extension = filename.split(".").pop()?.toUpperCase() || "";
    const simpleFilename = filename.split("/").pop() || "";
    const canPreview = EXTENSIONS_SUPPORT_PREVIEW.includes(extension);
    return (
        <Flex flexDirection={"column"} flex="0 0 20%" m={4} p={4} border="1px"
            borderColor={useColorModeValue('gray.200', 'gray.700')}>
            <Box>
                {canPreview
                    ? <Image src={`/files/${Buffer.from(filename).toString("base64")}/preview`} />
                    : <Box>this file cannot be previewed</Box>}
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