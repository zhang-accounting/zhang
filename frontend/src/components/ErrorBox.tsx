import { useQuery } from "@apollo/client";
import { AddIcon, ChevronLeftIcon, ChevronRightIcon } from "@chakra-ui/icons";
import { Box, Flex, Text, ButtonGroup, Button, IconButton } from "@chakra-ui/react";
import { ErrorEntity, ErrorListQuery, ERROR_LIST } from "../gql/errorList";
import Block from "./Block";
import { Connection } from "../models";

export default function ErrorBox() {
    const { data, loading, refetch } = useQuery<ErrorListQuery>(ERROR_LIST);

    const fetchNextPage = () => {
        refetch({
            cursor: data?.errors.pageInfo.endCursor
        })
    }
    const fetchPreviousPage = () => {
        const cursor  = parseInt(data!.errors.pageInfo.startCursor) - 11;
        if(cursor > 0) {
            refetch({
                cursor: (cursor).toString()
            })
        }else {
            refetch({cursor: "-1"})
        }
        
    }

    if (loading) return (<div> loading</div>);
    return (
        <Block title="errors">
            <Flex flexDirection={"column"}>
                {data?.errors.edges.map(edge => edge.node).map(error => (
                    <Box>
                        <Text whiteSpace={"nowrap"} textOverflow="ellipsis" overflow="hidden">{error.message}</Text>
                    </Box>
                ))}
            </Flex>

            <Flex justifyContent={"end"} mt={3}>
                <ButtonGroup size='sm' isAttached variant='outline'>
                    <IconButton disabled={!data?.errors.pageInfo.hasPreviousPage} onClick={fetchPreviousPage} aria-label='Add to friends' icon={<ChevronLeftIcon />} />
                    <IconButton disabled={!data?.errors.pageInfo.hasNextPage} onClick={fetchNextPage} aria-label='Add to friends' icon={<ChevronRightIcon />} />
                </ButtonGroup>
            </Flex>
        </Block>
    )
}