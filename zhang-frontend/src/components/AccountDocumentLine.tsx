import { Badge, Flex, Text } from "@chakra-ui/react";

interface Props {
    filename: string
    account: {
        name: string,
        status: string
    }
}

export default function Component({ filename, account }: Props) {

    const extension = filename.split(".").pop()?.toUpperCase();

    return (
        <Flex m={2} justifyContent="space-between">
            <Flex alignItems={"center"} alignContent="center">
                <Badge variant='outline'>{extension}</Badge>
                <Text mx={2}>{filename}</Text>
            </Flex>
            <Text mx={1}>{account.name}</Text>
        </Flex>
    )
}