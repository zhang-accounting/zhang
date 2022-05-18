import { Box, Flex, Tag, Text } from "@chakra-ui/react";
import { format } from "date-fns";
import { TransactionDto } from "../../gql/jouralList";
import Amount from "../Amount";
import Block from "../Block";
interface Props {
    data: TransactionDto
}
export default function TransactionPreview(props: Props) {

    return (
        <div>
            <Box mb={10}>
                <Box mx={1} my={2}>{format(new Date(props.data.timestamp * 1000), "yyyy-MM-dd hh:mm:ss")}</Box>
                <Box mx={1} my={2}><Text fontWeight={"bold"}>{props.data.payee}</Text></Box>
                <Box mx={1} my={2}>{props.data.narration}</Box>
                <Box mx={1} my={2}>
                    {props.data.tags.map(tag => <Tag borderRadius='full' variant="outline">#{tag}</Tag>)}
                    {props.data.links.map(link => <Tag borderRadius='full' variant="outline" colorScheme='blue'>^{link}</Tag>)}
                </Box>
            </Box>

            <Box mx={1} my={4}>
                <Block title="Postings">
                    <>
                        {props.data.postings.map((posting, idx) => <Flex key={idx} justifyContent="space-between">
                            <div>{posting.account.name}</div>
                            <div><Amount amount={posting.unit.number} currency={posting.unit.currency} /></div>
                        </Flex>)}
                    </>
                </Block>
            </Box>
            {
                props.data.metas.length > 0 && <Box mx={1} my={4}>
                    <Block title="Metas">
                        <Box>
                            {props.data.metas.filter((meta) => meta.key !== "document").map((meta, idx) => (
                                <Flex key={idx} justifyContent="space-between">
                                    <div>{meta.key}</div>
                                    <div>{meta.value}</div>
                                </Flex>
                            ))}
                        </Box>
                    </Block>
                </Box>
            }

            {
                props.data.metas.filter((meta)=>meta.key === "document").length>0 && <Box mx={1} my={4}>
                    <Block title={`${props.data.metas.filter((meta)=>meta.key === "document").length} Documents`}>
                        <Box>
                            {props.data.metas.filter((meta)=>meta.key === "document").map((meta, idx) => (
                                <Flex key={idx} justifyContent="space-between">
                                    <div>{meta.value}</div>
                                </Flex>
                            ))}
                        </Box>
                    </Block>
                </Box>
            }
        </div>
    )
}