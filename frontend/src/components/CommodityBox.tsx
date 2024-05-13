import { Group, Stack, Text } from '@mantine/core';
import React from 'react';
import Amount from './Amount';
import { useNavigate } from 'react-router';
import { createStyles } from '@mantine/emotion';

const useStyles = createStyles((theme, _, u) => ({
  card: {
    backgroundColor: 'transparent',
    padding: theme.spacing.sm,
    border: `2px solid ${theme.colors.gray[1]}`,
    borderRadius: theme.radius.sm,
    '&:hover': {
      border: `2px solid ${theme.colors[theme.primaryColor][6]}`,
    },
  },
  commodity: {
    cursor: 'pointer',
  },
}));

interface Props {
  name: string;
  total_amount: string;
  operating_currency: boolean;
  latest_price_amount?: string;
  latest_price_commodity?: string;
  latest_price_date?: string;
}

export default function CommodityBox(props: Props) {
  const { classes } = useStyles();

  const navigate = useNavigate();

  const onCommodityClick = (commodityName: string) => {
    navigate(commodityName);
  };
  return (
    <Stack className={classes.card} mt="sm">
      <Stack gap={'xs'}>
        <Group justify={'space-between'}>
          <Text className={classes.commodity} size="lg" fw={500} onClick={() => onCommodityClick(props.name)}>
            {props.name}
          </Text>
          <Amount amount={props.total_amount} currency="" />
        </Group>

        {props.latest_price_amount && (
          <Group justify={'space-between'}>
            <div></div>
            <Text size={'xs'} c={'dimmed'}>
              <Amount amount={1} currency={props.name} /> = <Amount amount={props.latest_price_amount} currency={props.latest_price_commodity!} />
            </Text>
          </Group>
        )}
      </Stack>
    </Stack>
  );
}
