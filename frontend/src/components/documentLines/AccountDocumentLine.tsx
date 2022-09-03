import { Badge, Flex, Text, Box, Image, useColorModeValue } from "@chakra-ui/react";
import { useNavigate } from "react-router";
import { AccountItem } from "../../gql/accountList";
import { Buffer } from "buffer";

export interface Props {
    filename: string,
    account?: AccountItem
}

export const EXTENSIONS_SUPPORT_PREVIEW = ['PNG', 'JPG', 'GIF']

export default function AccountDocumentLine({ filename, account }: Props) {
    let navigate = useNavigate();
    const extension = filename.split(".").pop()?.toUpperCase() || "";
    const simpleFilename = filename.split("/").pop() || "";
    const canPreview = EXTENSIONS_SUPPORT_PREVIEW.includes(extension);
    return (

        <Flex flexDirection={"column"} flex="0 0 20%" m={4} p={4} border="1px"
            borderColor={useColorModeValue('gray.200', 'gray.700')}>
            <Box>
                {canPreview
                ?<Image src={`/files/${Buffer.from(filename).toString("base64")}/preview`} />
                : <Box>this file cannot be previewed</Box>   }
            </Box>
            <Flex flexDirection={"column"} mt={2}>
                <Flex alignItems={"center"}>
                    <Badge variant='outline'>{extension}</Badge>
                    <Text mx={2}>{simpleFilename}</Text>
                </Flex>
                <Box>
                    <Text mx={1} cursor="pointer" onClick={() => navigate(`/accounts/${account?.name}`)}>{account?.name}</Text>
                </Box>
            </Flex>

        </Flex>
    )
}