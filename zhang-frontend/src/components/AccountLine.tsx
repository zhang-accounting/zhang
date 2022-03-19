import { Flex, Text, Box } from "@chakra-ui/react";
import { useNavigate } from "react-router";

export default function Component({ data }) {
    let navigate = useNavigate();

    const onNavigate = () => {
        if (data.isNode) {
            navigate(data.val.name)
        }
    }

    return (
        <Flex direction={"column"} py={1} px={2} border={"1px"} m={1} borderColor={"gray.200"} >
            <Text _hover={{ backgroundColor: "gray.200" }} onClick={onNavigate} cursor={data.isNode? "pointer":"default"}>{data.word}</Text>
            <Box pl={2}>
                {Object.keys(data.children).map(child => (
                    <Component data={data.children[child]} />
                ))}
            </Box>
        </Flex>
    )
}