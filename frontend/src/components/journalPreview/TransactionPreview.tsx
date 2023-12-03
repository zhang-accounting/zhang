import { Badge, Box, Group, SimpleGrid, Text, createStyles } from '@mantine/core';
import { format } from 'date-fns';
import { JournalTransactionItem } from '../../rest-model';
import Amount from '../Amount';
import DashLine from '../DashedLine';
import Section from '../Section';
import DocumentPreview from './DocumentPreview';
import AccountDocumentUpload from '../AccountDocumentUpload';
const useStyles = createStyles((theme) => ({
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
  data: JournalTransactionItem;
}
export default function TransactionPreview(props: Props) {
  const { classes } = useStyles();
  return (
    <div>
      <Section title="Transaction Info">
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
          <Text lineClamp={1}>Transaction</Text>
        </DashLine>
        <DashLine>
          <Text lineClamp={1} my="xs">
            Check Status
          </Text>
          <Text lineClamp={1}>
            {props.data.is_balanced ? (
              <Badge size="lg" color={'green'}>
                Pass
              </Badge>
            ) : (
              <Badge color={'red'}>UNBALANCED</Badge>
            )}
          </Text>
        </DashLine>
        <DashLine>
          <Text lineClamp={1} my="xs">
            Payee
          </Text>
          <Text lineClamp={1}>{props.data.payee}</Text>
        </DashLine>
        <DashLine>
          <Text lineClamp={1} my="xs">
            Narration
          </Text>
          <Text lineClamp={1}>{props.data.narration}</Text>
        </DashLine>
        {(props.data.links || []).length > 0 && (
          <DashLine>
            <Text lineClamp={1} my="xs">
              Links
            </Text>
            <Text lineClamp={1}>
              <Group mx={1} my={2} spacing="sm">
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
              <Group mx={1} my={2} spacing="sm">
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
      <Box mx={1} my={4}>
        <Section title={`${props.data.metas.filter((meta) => meta.key === 'document').length} Documents`}>
          <SimpleGrid
            cols={4}
            spacing="sm"
            breakpoints={[
              { maxWidth: 'md', cols: 3, spacing: 'md' },
              { maxWidth: 'sm', cols: 2, spacing: 'sm' },
              { maxWidth: 'xs', cols: 1, spacing: 'sm' },
            ]}
          >
            {props.data.metas
              .filter((meta) => meta.key === 'document')
              .map((meta, idx) => (
                <DocumentPreview key={idx} uri={meta.value} filename={meta.value} />
              ))}
            <AccountDocumentUpload url={`/api/transactions/${props.data.id}/documents`} />
          </SimpleGrid>
        </Section>
      </Box>
    </div>
  );
}
