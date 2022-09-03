import { Badge, Box, Flex, Image, Text, useColorModeValue } from "@chakra-ui/react";
import { Buffer } from "buffer";
import { AccountItem } from "../../gql/accountList";
import { TransactionDto } from "../../gql/jouralList";


export interface DocumentRenderItem {
    filename: string
    accounts: (AccountItem | undefined)[]
    transactions: (TransactionDto | undefined)[]
}


export interface Props extends DocumentRenderItem {
    
}

export const EXTENSIONS_SUPPORT_PREVIEW = ['PNG', 'JPG', 'GIF']

export default function AccountDocumentLine({ filename }: Props) {
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
            </Flex>

        </Flex>
    )
}