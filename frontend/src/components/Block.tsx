import { Box } from "@chakra-ui/react";
import { ReactElement } from "react";
interface Props {
    title: string,
    children: ReactElement | ReactElement[]
}
export default function Block({title, children}: Props) {
    return (
        <Box px={4} py={2} borderWidth={"1px"} borderColor={"gray.200"} borderRadius={3}>
            <Box mb={2} fontWeight={"bold"}>{title}</Box>
            <Box>{children}</Box>
        </Box>
    )
}