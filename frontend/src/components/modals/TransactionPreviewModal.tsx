import {ContextModalProps} from "@mantine/modals";
import JournalPreview from "../journalPreview/JournalPreview";
import {JournalItem} from "../../rest-model";

export const TransactionPreviewModal = ({ context, id, innerProps }: ContextModalProps<{ data: JournalItem }>) => (
    <>
        <JournalPreview data={innerProps.data} />
    </>
);