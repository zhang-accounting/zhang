import { Badge, Box, Group, Text } from '@mantine/core';
import { format } from 'date-fns';
import { JournalBlancePadItem } from '../../rest-model';
import Amount from '../Amount';
import DashLine from '../DashedLine';
import Section from '../Section';
import { createStyles } from '@mantine/emotion';

const useStyles = createStyles((theme, _, u) => ({
  amount: {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'end',
  },
  balance: {
    fontSize: theme.fontSizes.sm,
    color: theme.colors.gray[7],
  },
}));

interface Props {
  data: JournalBlancePadItem;
}

export default function BalancePadPreview(props: Props) {
  const { classes } = useStyles();
  return (
    <div>
      <Section title="Check Info">
        <DashLine>
          <Text lineClamp={1} my="xs">
            Datetime
          </Text>
          <Text lineClamp={1}>{format(new Date(props.data.datetime), 'yyyy-MM-dd HH:mm:ss')}</Text>
        </DashLine>

        <DashLine>
          <Text lineClamp={1} my="xs">
            Type
          </Text>
          <Text lineClamp={1}>Balance Pad</Text>
        </DashLine>
        <DashLine>
          <Text lineClamp={1} my="xs">
            Balance Account
          </Text>
          <Text lineClamp={1}>{props.data.postings[0].account}</Text>
        </DashLine>
        <DashLine>
          <Text lineClamp={1} my="xs">
            Pad Account
          </Text>
          <Text lineClamp={1}>{props.data.postings[1].account}</Text>
        </DashLine>
        {(props.data.links || []).length > 0 && (
          <DashLine>
            <Text lineClamp={1} my="xs">
              Links
            </Text>
            <Text lineClamp={1}>
              <Group mx={1} my={2} gap="sm">
                {(props.data.links || []).map((link) => (
                  <Badge key={link} size="lg" variant="dot">
                    {link}
                  </Badge>
                ))}
              </Group>
            </Text>
          </DashLine>
        )}

        {(props.data.tags || []).length > 0 && (
          <DashLine>
            <Text lineClamp={1} my="xs">
              Tags
            </Text>
            <Text lineClamp={1}>
              <Group mx={1} my={2} gap="sm">
                {(props.data.tags || []).map((tag) => (
                  <Badge key={tag} size="lg" color="orange" variant="dot">
                    {tag}
                  </Badge>
                ))}
              </Group>
            </Text>
          </DashLine>
        )}
      </Section>
      <Box mx={1} my={4}>
        <Section title="Postings">
          <>
            {props.data.postings.map((posting, idx) => (
              <DashLine key={idx}>
                <Text lineClamp={1} my="xs">
                  {posting.account}
                </Text>
                <div className={classes.amount}>
                  <Amount amount={posting.inferred_unit_number} currency={posting.inferred_unit_commodity} />
                  <div className={classes.balance}>
                    Balance: <Amount amount={posting.account_after_number} currency={posting.account_after_commodity} />
                  </div>
                </div>
              </DashLine>
            ))}
          </>
        </Section>
      </Box>

      {(props.data.metas ?? []).length > 0 && (
        <Section title="Metas">
          {props.data.metas
            .filter((meta) => meta.key !== 'document')
            .map((meta, idx) => (
              <DashLine key={idx}>
                <Text lineClamp={1} my="xs">
                  {meta.key}
                </Text>
                <Text lineClamp={1}>{meta.value}</Text>
              </DashLine>
            ))}
        </Section>
      )}
    </div>
  );
}
