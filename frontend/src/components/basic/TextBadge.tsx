import { createStyles } from '@mantine/emotion';

const useStyles = createStyles((theme, _, u) => ({
  badge: {
    display: 'inline-flex',
    padding: '0',
    paddingLeft: theme.spacing.xs,
    paddingRight: theme.spacing.xs,
    fontSize: theme.fontSizes.xs,
    [u.dark]: {
      color: theme.white,
      border: `1px solid ${theme.colors.dark[4]}`,
    },
    [u.light]: {
      color: theme.black,
      border: `1px solid ${theme.colors.gray[3]}`,
    },

    borderRadius: '99px',

  },
  clickableBadge: {
    cursor: 'pointer',
    '&:hover': {
      [u.dark]: { borderColor: theme.colors.dark[6] },
      [u.light]: { borderColor: theme.colors.gray[5] },

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
