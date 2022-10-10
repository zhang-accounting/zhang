import { Text, Space, Group } from '@mantine/core';
import { useNavigate } from 'react-router';
import AccountTrie from '../utils/AccountTrie';
import Amount from './Amount';

interface Props {
  data: AccountTrie;
  spacing: number;
}

export default function AccountLine({ data, spacing }: Props) {
  let navigate = useNavigate();

  const onNavigate = () => {
    if (data?.val?.name) {
      navigate(data?.val?.name);
    }
  };

  return (
    <>
      {data.isNode && (
        <tr>
          <td>
            <Group spacing="xs">
              <Space w={spacing * 10}></Space>
              <div onClick={onNavigate} style={{ cursor: 'pointer' }}>
                <Text>{data.val?.name.split(':').pop()}</Text>
                <Text color="dimmed" size="xs">
                  {data.val?.name}
                </Text>
              </div>
            </Group>
          </td>
          <td>{data.isNode && <Amount amount={data!.val!.snapshot.summary.number} currency={data!.val!.snapshot.summary.currency}></Amount>}</td>
        </tr>
      )}

      {Object.keys(data.children)
        .sort()
        .map((child, idx) => (
          <AccountLine key={idx} data={data.children[child]} spacing={data.isNode ? spacing + 1 : spacing} />
        ))}
    </>
  );
}
