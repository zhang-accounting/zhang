import { useDisclosure, useListState, useMediaQuery } from '@mantine/hooks';
import { format } from 'date-fns';
import { useState } from 'react';
// @ts-ignore
import { ActionIcon, Button, Code, Container, Divider, Grid, Group, Modal, Select, TextInput } from '@mantine/core';
import { DatePicker } from '@mantine/dates';
import { IconSquarePlus, IconTextPlus, IconTrashX } from '@tabler/icons';
import axios from "axios";
import useSWR from 'swr';
import { fetcher } from '..';
import { InfoForNewTransaction } from '../rest-model';
import DividerWithAction from './basic/DividerWithAction';

interface Posting {
    account: string | null;
    amount: string;
}

interface SelectItem {
    label: string;
    value: string;
    group?: string;
}

export default function NewTransactionButton() {
    const {data, error} = useSWR<InfoForNewTransaction>("/api/for-new-transaction", fetcher);



    const [isOpen, isOpenHandler] = useDisclosure(false);

    const isMobile = useMediaQuery('(max-width: 600px)');

    const [dateOnly] = useState(true);
    const [date, setDate] = useState<Date | null>(new Date());
    const [payee, setPayee] = useState<string | null>(null);
    const [narration, setNarration] = useState('');
    const [postings, postingsHander] = useListState<Posting>([
        {account: null, amount: ''},
        {account: null, amount: ''},
    ]);

    const [metas, metaHandler] = useListState<{ key: string, value: string }>([]);

    const preview = (): string => {
        const dateDisplay = format(date || 0, dateOnly ? 'yyyy-MM-dd' : 'yyyy-MM-dd HH:mm:ss');
        const narrationDisplay = narration.trim().length === 0 ? '' : ` ${JSON.stringify(narration.trim())}`;
        const postingDisplay = postings.map((posting) => `  ${posting.account} ${posting.amount}`).join('\n');
        const metaDisplay = metas.filter(meta => meta.key.trim() !== "" && meta.value.trim() !== "").map(meta => `  ${JSON.stringify(meta.key)}: ${JSON.stringify(meta.value)}`).join("\n");
        return `${dateDisplay} ${JSON.stringify(payee || '')}${narrationDisplay}\n${postingDisplay}\n${metaDisplay}`;
    };

    const valid = (): boolean => {
        return postings.every((posting) => posting.account !== null) &&
            postings.filter((posting) => posting.amount.trim().length === 0).length <= 1;
    };

    const onCreate = () => {
        axios.post(`/api/transactions`, {
            datetime: date?.toISOString(),
            payee: payee,
            narration: narration,
            postings: postings.map(it => ({
                account: it.account,
                unit: it.amount.trim() === '' ? null : {
                    number: it.amount.split(" ")[0],
                    commodity: it.amount.split(" ")[1]
                }
            })),
            tags: [],
            links: [],
            metas: metas
        })
            .then(res => {
                isOpenHandler.close();
                setDate(new Date());
                setPayee(null);
                setNarration('');
                postingsHander.setState([
                    {account: null, amount: ''},
                    {account: null, amount: ''},
                ]);
                metaHandler.setState([]);
            })
            .catch(function (error) {
                console.log(error);
            });
    }
    if (error) return <div>failed to load</div>
    if (!data) return <div>loading...</div>

    const payeeSelectItems: SelectItem[] = data.payee.map(item => {
        return {
            label: item,
            value: item,
        };
    })

    const accountItems = (data.account_name).map((singleAccountName) => {
        const type = singleAccountName.split(':')[0];
        return {
            label: singleAccountName,
            value: singleAccountName,
            group: type,
        };
    });

    return (
        <>
            <Button size="xs" leftIcon={<IconSquarePlus/>} onClick={() => isOpenHandler.open()}>
                NEW
            </Button>

            <Modal onClose={() => isOpenHandler.close()} opened={isOpen} size="xl" centered closeOnEscape
                   overflow="inside" title="New Transaction" fullScreen={isMobile}>
                <Container>
                    <Grid>
                        <Grid.Col sm={12} lg={4}>
                            <DatePicker placeholder="Transaction Date" value={date} onChange={setDate} withAsterisk/>
                        </Grid.Col>
                        <Grid.Col sm={12} lg={4}>
                            <Select
                                placeholder="Payee"
                                data={payeeSelectItems}
                                value={payee}
                                searchable
                                creatable
                                getCreateLabel={(query) => `+ Create ${query}`}
                                onChange={setPayee}
                            />
                        </Grid.Col>
                        <Grid.Col sm={12} lg={4}>
                            <TextInput placeholder="Narration" value={narration}
                                       onChange={(e) => setNarration(e.target.value)}/>
                        </Grid.Col>
                    </Grid>

                    <DividerWithAction value="Postings" icon={<IconTextPlus/>}
                                       onActionClick={() => postingsHander.append({
                                           account: null,
                                           amount: ''
                                       })}></DividerWithAction>

                    {postings.map((posting, idx) => (
                        <Grid align="center" key={idx}>
                            <Grid.Col span={8}>
                                <Select searchable placeholder="Account" data={accountItems} value={posting.account}
                                        onChange={(e) => postingsHander.setItemProp(idx, "account", e)}/>
                            </Grid.Col>
                            <Grid.Col span={3}>
                                <TextInput placeholder="Amount" value={posting.amount}
                                           onChange={(e) => postingsHander.setItemProp(idx, "amount", e.target.value)}/>
                            </Grid.Col>
                            <Grid.Col span={1}>
                                <ActionIcon disabled={postings.length <= 2} onClick={() => postingsHander.remove(idx)}>
                                    <IconTrashX/>
                                </ActionIcon>
                            </Grid.Col>
                        </Grid>
                    ))}

                    <DividerWithAction value="Metas" icon={<IconTextPlus/>} onActionClick={() => {
                        metaHandler.append({key: "", value: ""})
                    }}></DividerWithAction>

                    {metas.map((meta, idx) => (
                        <Grid align="center" key={idx}>
                            <Grid.Col span={4}>
                                <TextInput placeholder="key" value={meta.key}
                                           onChange={(e) => metaHandler.setItemProp(idx, "key", e.target.value)}/>
                            </Grid.Col>
                            <Grid.Col span={7}>
                                <TextInput placeholder="value" value={meta.value}
                                           onChange={(e) => metaHandler.setItemProp(idx, "value", e.target.value)}/>
                            </Grid.Col>
                            <Grid.Col span={1}>
                                <ActionIcon onClick={() => metaHandler.remove(idx)}>
                                    <IconTrashX/>
                                </ActionIcon>
                            </Grid.Col>
                        </Grid>
                    ))}
                    <Divider label="Preview" size="xs" my="md"></Divider>
                    <Code block>{preview()}</Code>

                    <Group position="right" my="md">
                        <Button variant="outline" onClick={isOpenHandler.close}>
                            Cancel
                        </Button>
                        <Button mr={3} onClick={onCreate} disabled={!valid()}>
                            Save
                        </Button>
                    </Group>
                </Container>
            </Modal>
        </>
    );
}
