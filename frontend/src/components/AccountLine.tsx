import { Text, Space, Group, ActionIcon } from '@mantine/core';
import { useLocalStorage } from '@mantine/hooks';
import { IconChevronDown, IconChevronRight } from '@tabler/icons';
import { useNavigate } from 'react-router';
import AccountTrie from '../utils/AccountTrie';
import Amount from './Amount';

interface Props {
  data: AccountTrie;
  spacing: number;
}

export default function AccountLine({ data, spacing }: Props) {
  let navigate = useNavigate();
  const [isShow, setCollapse] = useLocalStorage({ key: `account-collapse-${data.path}`, defaultValue: false });

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
                {isShow ? <IconChevronDown size={14} /> : <IconChevronRight size={14} />}
              </ActionIcon>
            ) : (
              <Space w={22}></Space>
            )}
            <div onClick={onNavigate} style={{ cursor: 'pointer' }}>
              <Text>{data.word}</Text>
              {data.val && (
                <Text color="dimmed" size="xs">
                  {data.val?.name}
                </Text>
              )}
            </div>
          </Group>
        </td>
        <td style={{ textAlign: 'end' }}>
          {data?.val?.snapshot && <Amount amount={data!.val!.snapshot.summary.number} currency={data!.val!.snapshot.summary.currency}></Amount>}
        </td>
      </tr>

      {isShow &&
        Object.keys(data.children)
          .sort()
          .map((child) => <AccountLine key={data.children[child].path} data={data.children[child]} spacing={spacing + 1} />)}
    </>
  );
}
