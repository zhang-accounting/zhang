import { Box, Flex,Text } from "@chakra-ui/react";
interface Props {
    title: string
}
export default function Component({title}: Props) {
    return (
        <Box m={2} px={4} py={2} borderWidth={"1px"} borderColor={"gray.200"} borderRadius={3}>
            <Box mb={2} fontWeight={"bold"}>{title}</Box>
            <Box>content</Box>
        </Box>
    )
}