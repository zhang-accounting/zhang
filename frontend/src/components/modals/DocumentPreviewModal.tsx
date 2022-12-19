import { Image } from "@mantine/core";
import { ContextModalProps } from "@mantine/modals";
import { Buffer } from "buffer";
import { serverBaseUrl } from "../../index";

export const DocumentPreviewModal = ({ context, id, innerProps }: ContextModalProps<{ path: string, filename: string }>) => (
    <>
        {/*<Text size="sm">{innerProps.modalBody}</Text>*/}
        <Image src={`${serverBaseUrl}/api/documents/${Buffer.from(innerProps.path).toString('base64')}`}
        height="50vh" fit="contain"/>
    </>
);