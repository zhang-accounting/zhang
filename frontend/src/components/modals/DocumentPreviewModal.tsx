import {Button, Image, Text} from "@mantine/core";
import { ContextModalProps } from "@mantine/modals";
import {serverBaseUrl} from "../../index";
import {Buffer} from "buffer";

export const DocumentPreviewModal = ({ context, id, innerProps }: ContextModalProps<{ path: string, filename: string }>) => (
    <>
        {/*<Text size="sm">{innerProps.modalBody}</Text>*/}
        <Image src={`${serverBaseUrl}/api/documents/${Buffer.from(innerProps.path).toString('base64')}`}
        height="50vh" fit="contain"/>
    </>
);