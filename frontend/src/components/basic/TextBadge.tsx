import { createStyles } from '@mantine/core';

const useStyles = createStyles((theme) => ({
  badge: {
    display: 'inline-flex',
    padding: '0',
    paddingLeft: theme.spacing.xs,
    paddingRight: theme.spacing.xs,
    fontSize: theme.fontSizes.xs,
    color: theme.colorScheme === 'dark' ? theme.white : theme.black,
    borderRadius: '99px',
    border: `1px solid ${theme.colorScheme === 'dark' ? theme.colors.dark[4] : theme.colors.gray[3]}`,
  },
  clickableBadge: {
    cursor: 'pointer',
    '&:hover': {
      borderColor: theme.colorScheme === 'dark' ? theme.colors.dark[6] : theme.colors.gray[5],
    },
  },
}));

interface Props {
  children: string;
  onClick?: () => void;
}

export function TextBadge(props: Props) {
  const { classes } = useStyles();

  return (
    <div onClick={props.onClick} className={`${classes.badge} ${!!props.onClick ? classes.clickableBadge : ''}`}>
      {props.children}
    </div>
  );
}
