import {Button, Group, Modal, Pagination, Stack, Text} from '@mantine/core';
import CodeMirror from '@uiw/react-codemirror';
import {useState} from 'react';
import {useTranslation} from 'react-i18next';
import useSWR from "swr";
import {fetcher} from "../index";
import {LedgerError, Pageable} from "../rest-model";

export default function ErrorBox() {
    const {t} = useTranslation();
    const [isOpen, setIsOpen] = useState(false);
    const [page, setPage] = useState(1);
    const {data, error} = useSWR<Pageable<LedgerError>>(`/api/errors?page=${page}`, fetcher);

    const [selectError, setSelectError] = useState<LedgerError | null>(null);
    const [selectErrorContent, setSelectErrorContent] = useState<string>('');

    if (error) return <div>failed to load</div>;
    if (!data) return <>loading</>;
    const {total_page, records, current_page} = data;

    const toggleError = (error: LedgerError) => {
        setSelectError(error);
        setSelectErrorContent(error.span.content);
        setIsOpen(true);
    };
    const saveErrorModfiyData = () => {
        //   modifyFile({
        //     variables: {
        //       file: selectError?.span.filename,
        //       content: selectErrorContent,
        //       start: selectError?.span.start,
        //       end: selectError?.span.end,
        //     },
        //   });
        setIsOpen(false);
    };
    const onModalReset = () => {
        setSelectErrorContent(selectError?.span.content || '');
    };
    return (
        <>
            <Modal
                size="lg"
                centered
                opened={isOpen}
                onClose={() => setIsOpen(false)}
                title={`${selectError?.span.filename}:${selectError?.span.start}:${selectError?.span.end}`}>
                <Text>{t(selectError?.error.type || "")}</Text>
                <CodeMirror
                    value={selectErrorContent}
                    height="20vh"
                    width="100%"
                    onChange={(value) => {
                        setSelectErrorContent(value);
                    }}
                />
                <Group position="right">

                    <Button onClick={onModalReset} variant="default">
                        {t('Reset')}
                    </Button>
                    <Button onClick={saveErrorModfiyData} variant="default">
                        {t('Save')}
                    </Button>
                </Group>
            </Modal>
            <Stack>

                {records
                    .map((error, idx) => (
                        <Text key={idx} onClick={() => toggleError(error)}>
                            {t(error.error.type)}
                        </Text>
                    ))}

                <Pagination mt="xs" total={total_page} page={current_page} onChange={setPage} position="center"/>

            </Stack>
        </>
    );
}