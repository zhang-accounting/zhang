import {Text, Space, Group, ActionIcon, Badge, Stack} from '@mantine/core';
import {useLocalStorage} from '@mantine/hooks';
import {IconChevronDown, IconChevronRight} from '@tabler/icons';
import {useNavigate} from 'react-router';
import AccountTrie from '../utils/AccountTrie';
import Amount from './Amount';
import {AccountStatus} from "../rest-model";

interface Props {
    data: AccountTrie;
    spacing: number;
}

export default function AccountLine({data, spacing}: Props) {
    let navigate = useNavigate();
    const [isShow, setCollapse] = useLocalStorage({key: `account-collapse-${data.path}`, defaultValue: false});

    const onNavigate = () => {
        if (data?.val?.name) {
            navigate(data?.val?.name);
        }
    };
    const hasChildren = Object.keys(data.children).length > 0;

    return (
        <>
            <tr>
                <td>
                    <Group spacing="xs">
                        <Space w={spacing * 22}></Space>
                        {hasChildren ? (
                            <ActionIcon size="sm" variant="transparent" onClick={() => setCollapse(!isShow)}>
                                {isShow ? <IconChevronDown size={14}/> : <IconChevronRight size={14}/>}
                            </ActionIcon>
                        ) : (
                            <Space w={22}></Space>
                        )}
                        <div onClick={onNavigate} style={{cursor: 'pointer'}}>
                            <Group>
                                <Text>{data.word}</Text>
                                {data.val?.status === AccountStatus.Close && (
                                    <Badge size="xs" color="red" variant="dot">
                                        {data.val?.status}
                                    </Badge>
                                )}
                            </Group>

                            {data.val && (
                                <Text color="dimmed" size="xs">
                                    {data.val?.name}
                                </Text>
                            )}
                        </div>
                    </Group>
                </td>
                <td style={{textAlign: 'end'}}>
                    <Stack spacing="xs">
                        {Object.entries(data.val?.commodities ?? {}).map(([key,value]) => <Amount
                            amount={value}
                            currency={key}></Amount>)}

                    </Stack>
                </td>
            </tr>

            {isShow &&
                Object.keys(data.children)
                    .sort()
                    .map((child) => <AccountLine key={data.children[child].path} data={data.children[child]}
                                                 spacing={spacing + 1}/>)}
        </>
    );
}
