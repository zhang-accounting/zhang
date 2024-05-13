import { ActionIcon, Badge, Divider, Group, HoverCard, Space, Stack, Table, Text } from '@mantine/core';
import { useLocalStorage } from '@mantine/hooks';
import { IconChevronDown, IconChevronRight } from '@tabler/icons-react';
import { useNavigate } from 'react-router';
import { AccountStatus } from '../rest-model';
import AccountTrie from '../utils/AccountTrie';
import Amount from './Amount';
import { createStyles } from '@mantine/emotion';

const useStyles = createStyles((theme, _, u) => ({
  leaf: {
    cursor: 'pointer',
  },
  nonLeaf: {},
  leafAmount: {},
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

  const haveMultipleCommodity = Object.keys(data.amount.data).length > 1;
  const onNavigate = () => {
    if (data?.val?.name) {
      navigate(data?.val?.name);
    }
  };
  const hasChildren = Object.keys(data.children).length > 0;

  return (
    <>
      <Table.Tr>
        <Table.Td>
          <div style={{ display: 'flex' }}>
            <Space w={spacing * 22}></Space>
            {hasChildren ? (
              <ActionIcon size="sm" color="gray" variant="transparent" onClick={() => setCollapse(!isShow)}>
                {isShow ? <IconChevronDown size={14} /> : <IconChevronRight size={14} />}
              </ActionIcon>
            ) : (
              <Space w={22}></Space>
            )}
            <div onClick={onNavigate} className={data.isLeaf ? classes.leaf : classes.nonLeaf}>
              <Group>
                <span>{data.val?.alias ?? data.word}</span>
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
        </Table.Td>
        <Table.Td>
          <Group justify="right">
            {haveMultipleCommodity ? (
              <HoverCard width={280} shadow="md" withArrow position="left">
                <HoverCard.Target>
                  <Group gap="xs" className={data.isLeaf ? classes.leafAmount : classes.nonLeafAmount}>
                    <Text>≈</Text> <Amount amount={data.amount.total} currency={data.amount.commodity}></Amount>
                  </Group>
                </HoverCard.Target>
                <HoverCard.Dropdown>
                  <Stack gap="xs">
                    {Object.entries(data.amount.data).map(([key, value]) => (
                      <Group justify="space-between">
                        <Text>+</Text>
                        <Amount amount={value} currency={key}></Amount>
                      </Group>
                    ))}
                    <Divider variant="dashed" labelPosition="left" />
                    <Group justify="space-between">
                      <Text>=</Text>
                      <Amount amount={data.amount.total} currency={data.amount.commodity}></Amount>
                    </Group>
                  </Stack>
                </HoverCard.Dropdown>
              </HoverCard>
            ) : (
              <Group gap="xs" className={data.isLeaf ? classes.leafAmount : classes.nonLeafAmount}>
                <Amount amount={data.amount.total} currency={data.amount.commodity}></Amount>
              </Group>
            )}
          </Group>
        </Table.Td>
      </Table.Tr>

      {isShow &&
        Object.keys(data.children)
          .sort()
          .map((child) => <AccountLine key={data.children[child].path} data={data.children[child]} spacing={spacing + 1} />)}
    </>
  );
}
