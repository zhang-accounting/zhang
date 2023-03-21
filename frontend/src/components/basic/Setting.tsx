import { Text } from "@mantine/core";
import { ReactElement } from "react";


interface Props {
    title: string,
    value?: string,
    uppercase?: boolean,
    children?: ReactElement | ReactElement[];
}
export function Setting({ title, value, children, uppercase }: Props) {
    const shouldUppercase = uppercase ?? false;
    return (<div>
        <Text color="gray" fw="700" fz="sm">{shouldUppercase ? title.toUpperCase() : title}</Text>
        {value && <Text>{value}</Text>}
        {children}
    </div>)
}