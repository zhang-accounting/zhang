import { Box, Button, Code, Flex, Input, Modal, ModalBody, ModalCloseButton, ModalContent, ModalFooter, ModalHeader, ModalOverlay, Text, useDisclosure } from "@chakra-ui/react";
import { useState } from "react";

import Select from 'react-select';



export default function Component({ }) {
    const { isOpen, onOpen, onClose } = useDisclosure()
    const options = [
        { value: 'chocolate', label: 'Chocolate' },
        { value: 'strawberry', label: 'Strawberry' },
        { value: 'vanilla', label: 'Vanilla' },
    ];
    const [selectedOption, setSelectedOption] = useState(null);

    const [date, setDate] = useState("");
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
        const narrationDisplay = narration.trim().length === 0 ? "" : ` "${escape(narration.trim())}"`;
        const postingDisplay = postings.map(posting => `  ${posting.account?.value} ${posting.amount}`).join("\n");
        return `${date} "${escape(payee)}"${narrationDisplay}\n${postingDisplay}`
    }


    return (
        <>
            <Button onClick={onOpen}>Trigger modal</Button>

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
                                        <Input placeholder='Flushed' />
                                    </Box>
                                    <Box m={1}>
                                        <Input placeholder='Payee' value={payee} onChange={e => setPayee(e.target.value)} />
                                    </Box>
                                    <Box m={1}>
                                        <Input placeholder='Narration' value={narration} onChange={e => setNarration(e.target.value)} />
                                    </Box>
                                </Flex>
                                {postings.map((posting, idx) => (
                                    <Flex m={1} key={idx}>
                                        <Box w='80%'>
                                            <Select
                                                isClearable
                                                value={posting.account}
                                                onChange={(value) => updatePostingAccount(idx, value)}
                                                options={options}
                                            />
                                        </Box>
                                        <Box ml={2}>
                                            <Input placeholder='Amount' value={posting.amount} onChange={(e) => updatePostingAmount(idx, e.target.value)} />
                                        </Box>

                                    </Flex>
                                ))}
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
                        <Button colorScheme='blue' mr={3}>
                            Save
                        </Button>
                        <Button onClick={onClose}>Cancel</Button>
                    </ModalFooter>
                </ModalContent>
            </Modal>
        </>
    )
}