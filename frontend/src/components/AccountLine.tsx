import { Text, Space, Group, ActionIcon, Badge, Stack, createStyles } from '@mantine/core';
import { useLocalStorage } from '@mantine/hooks';
import { IconChevronDown, IconChevronRight } from '@tabler/icons';
import { useNavigate } from 'react-router';
import AccountTrie from '../utils/AccountTrie';
import Amount from './Amount';
import { AccountStatus } from '../rest-model';


const useStyles = createStyles((theme) => ({
  leafAmount: {
  },
  nonLeafAmount: {
    color: theme.colors.gray[5],
  },

}));

interface Props {
  data: AccountTrie;
  spacing: number;
}

export default function AccountLine({ data, spacing }: Props) {
  let navigate = useNavigate();
  const { classes } = useStyles();
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
          <div style={{ display: 'flex' }}>
            <Space w={spacing * 22}></Space>
            {hasChildren ? (
              <ActionIcon size="sm" variant="transparent" onClick={() => setCollapse(!isShow)}>
                {isShow ? <IconChevronDown size={14} /> : <IconChevronRight size={14} />}
              </ActionIcon>
            ) : (
              <Space w={22}></Space>
            )}
            <div onClick={onNavigate} style={{ cursor: 'pointer' }}>
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
          </div>
        </td>
        <td>
          <Group position="right">
            <Stack spacing="xs" className={data.isLeaf ? classes.leafAmount : classes.nonLeafAmount}>
              {Object.entries(data.amount.data).map(([key, value]) => (
                <Amount amount={value} currency={key}></Amount>
              ))}
            </Stack>
          </Group>
        </td>
      </tr>

      {isShow &&
        Object.keys(data.children)
          .sort()
          .map((child) => <AccountLine key={data.children[child].path} data={data.children[child]} spacing={spacing + 1} />)}
    </>
  );
}
