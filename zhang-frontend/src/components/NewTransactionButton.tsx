import { Box, Button, Checkbox, Code, Flex, Input, Modal, ModalBody, ModalCloseButton, ModalContent, ModalFooter, ModalHeader, ModalOverlay, Text, useBoolean, useDisclosure } from "@chakra-ui/react";
import React, { useState } from "react";

import Select from 'react-select';

import DateTimePicker from 'react-datetime-picker';
import { format } from "date-fns";
import { gql, useMutation, useQuery } from "@apollo/client";


export default function Component({ }) {

    const accountInfo = useQuery(gql`
    query NEW_TRANSACTION_MODAL_DATA {
        accounts {
          name
        }
      }
    `)

    const [appendData, _] = useMutation(gql`
    mutation APPEND_DATA($date: Int, $content: String) {
        appendData(date: $date, content: $content) 
    }
    `, {
        refetchQueries: ["FILE_LIST", "SINGLE_FILE_ENTRY", "JOURNAL_LIST", "BAR_STATISTIC"]
    })

    const { isOpen, onOpen, onClose } = useDisclosure()
    const [dateOnly, setDateOnly] = useBoolean(false);

    const [date, setDate] = useState(new Date());
    const [payee, setPayee] = useState("");
    const [narration, setNarration] = useState("");
    const [postings, setPostings] = useState([
        { account: null, amount: "" },
        { account: null, amount: "" }
    ])

    const updatePostingAccount = (idx, account) => {
        const clonedPostings = [...postings];
        clonedPostings[idx].account = account;
        setPostings(clonedPostings);
    }
    const updatePostingAmount = (idx, amount) => {
        const clonedPostings = [...postings];
        clonedPostings[idx].amount = amount;
        setPostings(clonedPostings);
    }

    const preview = (): string => {
        const dateDisplay = format(date, dateOnly ? "yyyy-MM-dd" : "yyyy-MM-dd HH:mm:ss");
        const narrationDisplay = narration.trim().length === 0 ? "" : ` ${JSON.stringify(narration.trim())}`;
        const postingDisplay = postings.map(posting => `  ${posting.account?.value} ${posting.amount}`).join("\n");
        return `${dateDisplay} ${JSON.stringify(payee)}${narrationDisplay}\n${postingDisplay}`
    }

    const valid = (): boolean => {
        return postings.every(posting => posting.account !== null) &&
            postings.filter(posting => posting.amount.trim().length === 0).length <= 1
    }
    const newPosting = () => {
        const newPostings = [...postings];
        newPostings.push({ account: null, amount: "" });
        setPostings(newPostings);
    }
    const handleDeletePosting = (targetIdx: number) => {
        setPostings(postings.filter((_, idx) => idx !== targetIdx));
    }
    const save = () => {
        appendData({
            variables: { date: Math.round(date.getTime() / 1000), content: `\n${preview()}\n` }
        });
        onClose();
        setDate(new Date);
        setPayee("");
        setNarration("");
        setPostings([
            { account: null, amount: "" },
            { account: null, amount: "" }
        ]);
    }

    if (accountInfo.loading) return <p>Loading...</p>;
    if (accountInfo.error) return <p>Error :(</p>;

    const accountSelectItems = Object.values(accountInfo.data.accounts.reduce((ret, singleAccountInfo) => {
        const type = singleAccountInfo.name.split(":")[0];
        const item = { label: singleAccountInfo.name, value: singleAccountInfo.name };
        ret[type] = ret[type] || { label: type.toUpperCase(), options: [] };
        ret[type].options.push(item);
        return ret;
    }, {})).sort();

    return (
        <>
            <Button onClick={onOpen}>New Transaction</Button>

            <Modal onClose={onClose} isOpen={isOpen} isCentered size="3xl">
                <ModalOverlay />
                <ModalContent>
                    <ModalHeader>New Transaction</ModalHeader>
                    <ModalCloseButton />
                    <ModalBody>
                        <Flex direction="column">
                            <Box>
                                <Flex m={1}>
                                    <Box m={1}>
                                        <DateTimePicker onChange={setDate} value={date} />
                                    </Box>
                                    <Box m={1}>
                                        <Input placeholder='Payee' value={payee} onChange={e => setPayee(e.target.value)} />
                                    </Box>
                                    <Box m={1}>
                                        <Input placeholder='Narration' value={narration} onChange={e => setNarration(e.target.value)} />
                                    </Box>
                                </Flex>
                                <Flex>
                                    <Button onClick={newPosting}>new Posting</Button>
                                </Flex>
                                {postings.map((posting, idx) => (
                                    <Flex m={1} key={idx}>
                                        <Box w='80%'>
                                            <Select
                                                isClearable
                                                isSearchable
                                                value={posting.account}
                                                onChange={(value) => updatePostingAccount(idx, value)}
                                                options={accountSelectItems}
                                            />
                                        </Box>
                                        <Box ml={2}>
                                            <Input placeholder='Amount' value={posting.amount} onChange={(e) => updatePostingAmount(idx, e.target.value)} />
                                        </Box>
                                        <Box ml={2}>
                                            <Button disabled={postings.length <= 2} onClick={() => handleDeletePosting(idx)}>Delete</Button>
                                        </Box>

                                    </Flex>
                                ))}
                                <Flex>
                                    <Checkbox checked={dateOnly} onChange={setDateOnly.toggle}>Date Only</Checkbox>
                                </Flex>
                            </Box>
                            <Box>
                                <Box>preview</Box>
                                <Box bg={"gray.100"} p={4}>
                                    <pre> <code>{preview()}</code></pre>
                                </Box>
                            </Box>
                        </Flex>


                    </ModalBody>
                    <ModalFooter>
                        <Button colorScheme='blue' mr={3} onClick={save} disabled={!valid()}>
                            Save
                        </Button>
                        <Button onClick={onClose}>Cancel</Button>
                    </ModalFooter>
                </ModalContent>
            </Modal>
        </>
    )
}