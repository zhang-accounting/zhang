import { useMutation, useQuery } from "@apollo/client";
import { ChevronLeftIcon, ChevronRightIcon } from "@chakra-ui/icons";
import { Box, Flex, Text, ButtonGroup, Button, IconButton, Modal, ModalOverlay, ModalContent, ModalHeader, ModalCloseButton, ModalBody, ModalFooter, useDisclosure } from "@chakra-ui/react";
import { ErrorEntity, ErrorListQuery, ERROR_LIST } from "../gql/errorList";
import Block from "./Block";
import { useState } from "react";
import CodeMirror from '@uiw/react-codemirror';
import { MODIFY_FILE } from "../gql/modifyFile";

export default function ErrorBox() {
    const { isOpen, onOpen, onClose } = useDisclosure();
    const { data, loading, refetch } = useQuery<ErrorListQuery>(ERROR_LIST);
    const [selectError, setSelectError] = useState<ErrorEntity | null>(null);
    const [selectErrorContent, setSelectErrorContent] = useState<string>("");

    const [modifyFile] = useMutation(MODIFY_FILE, {
        update: (proxy) => {
            proxy.evict({ fieldName: `journals` })
            proxy.evict({ fieldName: `errors` })
            console.log("proxy", proxy);
        }
    });

    const toggleError = (error: ErrorEntity) => {
        setSelectError(error);
        setSelectErrorContent(error.span.content);
        onOpen();
    }
    const saveErrorModfiyData = () => {
        modifyFile({
            variables: {
                file: selectError?.span.filename,
                content: selectErrorContent,
                start: selectError?.span.start,
                end: selectError?.span.end
            }
        })
        onClose()
    }
    const onModalReset = () => {
        setSelectErrorContent(selectError?.span.content || "")
    }
    const fetchNextPage = () => {
        refetch({
            cursor: data?.errors.pageInfo.endCursor
        })
    }
    const fetchPreviousPage = () => {
        const cursor = parseInt(data!.errors.pageInfo.startCursor) - 11;
        if (cursor > 0) {
            refetch({
                cursor: (cursor).toString()
            })
        } else {
            refetch({ cursor: "-1" })
        }
    }

    if (loading) return (<div> loading</div>);
    return (
        <>
            <Modal isOpen={isOpen} size={"4xl"} onClose={onClose}>
                <ModalOverlay />
                <ModalContent>
                    <ModalHeader>{selectError?.span.filename}:{selectError?.span.start}:{selectError?.span.end}</ModalHeader>
                    <ModalCloseButton />
                    <ModalBody>
                        <Text>{selectError?.message}</Text>
                        <CodeMirror
                            value={selectErrorContent}
                            height="20vh"
                            width="100%"
                            onChange={(value) => {
                                setSelectErrorContent(value)
                            }}
                        />
                    </ModalBody>

                    <ModalFooter>
                        <Button colorScheme='blue' mr={3} disabled={selectErrorContent === selectError?.span.content} onClick={saveErrorModfiyData}>
                            Modify
                        </Button>

                        <Button variant='ghost' onClick={onModalReset}>Reset</Button>
                    </ModalFooter>
                </ModalContent>
            </Modal>
            <Block title={`${data?.errorLength} errors`}>
                <Flex flexDirection={"column"}>
                    {data?.errors.edges.map(edge => edge.node).map(error => (
                        <Box onClick={() => toggleError(error)} cursor="pointer" my={1}>
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
        </>

    )
}