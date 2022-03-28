import { Button, Flex, Input, InputGroup, InputLeftAddon, InputRightElement, Text } from "@chakra-ui/react";
import Amount from "./Amount";
interface Props {

}
export default function Component({ }: Props) {

    const submitCheck = () => {

    }
    return (
        <InputGroup size='md'>
            <Input
                pr='4.5rem'
                type='text'
                placeholder='Balance Amount'
            />
            <InputRightElement width='4.5rem' px={2}>
                <Button h='1.75rem' size='sm' onClick={submitCheck}>
                    Check
                </Button>
            </InputRightElement>
        </InputGroup>
    )
}