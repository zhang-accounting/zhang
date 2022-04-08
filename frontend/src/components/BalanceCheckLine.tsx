import { Badge, Box, Flex, Text } from "@chakra-ui/react";
import { BalanceCheckDto } from "../gql/jouralList";
interface Props {
    data: BalanceCheckDto
}
export default function BalanceCheckLine({ data }: Props) {

    return (
        <Flex minH={"2rem"} mx={'auto'} py={0.5} px={{ base: 2, sm: 12, md: 17 }} borderBottom='1px' borderColor={"gray.200"} _hover={{ backgroundColor: "gray.200" }}
            alignItems={"center"} fontSize={"smaller"}>
            <Box mr={2}>
                <Text>{data.date}</Text>
            </Box>
            <Box mr={2}>
                <Badge variant='outline' colorScheme={data.isBalanced ? "green" : "red"}>BC</Badge>
            </Box>
            <Flex flex='1' overflow={"hidden"}>
                <Text mr={2}>{data.account?.name}</Text>
            </Flex>

            <Flex alignItems={"center"} >
                <Text mx={2}>{data.balanceAmount.number} {data.balanceAmount.currency}</Text>
                {!data.isBalanced &&
                    <Flex direction={"column"} fontSize={"smaller"}>
                        <Flex alignContent="space-between" justifyContent={"space-between"}>
                            <Text mx={2}>current:</Text>
                            <Text mx={2}>{data.currentAmount.number} {data.currentAmount.currency}</Text>
                        </Flex>
                        <Flex alignContent="space-between" justifyContent={"space-between"}>
                            <Text mx={2}>distance:</Text>
                            <Text mx={2}>{data.distance.number} {data.distance.currency}</Text>
                        </Flex>
                    </Flex>
                }

            </Flex>
        </Flex>
    )
}