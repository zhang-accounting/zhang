import { Badge, Box, Flex, Text } from "@chakra-ui/react";

export default function Component({ data }) {
    return (
        <Flex mx={'auto'} py={0.5} px={{ base: 2, sm: 12, md: 17 }} borderBottom='1px' borderColor={"gray.200"} _hover={{ backgroundColor: "gray.200" }}
            alignItems={"center"} fontSize={"smaller"}>
            <Box mr={2}>
                <Text>{data.date}</Text>
            </Box>
            <Box mr={2}>
                <Badge>TN</Badge>
            </Box>
            <Flex flex='1' overflow={"hidden"}>
                <Text fontWeight={500} mr={2}>{data.payee}</Text>
                <Text>{data.narration}</Text>
            </Flex>
            <Flex direction={"column"} >
                {data.postings.map(posting => (
                    <Flex alignContent="space-between" justifyContent={"space-between"}>
                        <Text mx={2}>{posting?.account?.name}</Text>
                        <Text align={"right"} mx={2}>{posting.unit.number} {posting.unit.currency}</Text>
                    </Flex>
                ))}
            </Flex>
        </Flex>
    )
}