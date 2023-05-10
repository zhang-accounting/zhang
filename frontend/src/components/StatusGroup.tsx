import { createStyles, SimpleGrid, Text } from '@mantine/core';
import Amount from './Amount';

const useStyles = createStyles((theme) => ({
  root: {
    backgroundImage: `linear-gradient(-60deg, ${theme.colors[theme.primaryColor][4]} 0%, ${theme.colors[theme.primaryColor][7]} 100%)`,
    padding: theme.spacing.xl,
    borderRadius: theme.radius.sm,
  },

  title: {
    color: theme.white,
    textTransform: 'uppercase',
    fontWeight: 700,
    fontSize: theme.fontSizes.sm,
  },

  count: {
    color: theme.white,
    fontSize: 18,
    lineHeight: 1,
    fontWeight: 700,
    marginBottom: theme.spacing.md,
    fontFamily: `Greycliff CF, ${theme.fontFamily}`,
  },

  description: {
    color: theme.colors[theme.primaryColor][0],
    fontSize: theme.fontSizes.sm,
    marginTop: 4,
  },

  stat: {
    flex: 1,
  },
}));

interface StatsGroupProps {
  data: { title: string; amount?: string; currency?: string; number?: number }[];
}

export default function StatsGroup({ data }: StatsGroupProps) {
  const { classes } = useStyles();
  const stats = data.map((stat) => (
    <div key={stat.title} className={classes.stat}>
      <Text className={classes.count}>
        {stat.number !== undefined ? <Text>{stat.number}</Text> : <Amount amount={stat.amount!} currency={stat.currency!} />}
      </Text>
      <Text className={classes.title}>{stat.title}</Text>
    </div>
  ));
  return (
    <SimpleGrid
      cols={data.length}
      breakpoints={[
        { maxWidth: 'md', cols: 2, spacing: 'sm' },
        { maxWidth: 'sm', cols: 2, spacing: 'sm' },
        { maxWidth: 'xs', cols: 1, spacing: 'sm' },
      ]}
      className={classes.root}>
      {stats}
    </SimpleGrid>
  );
}
