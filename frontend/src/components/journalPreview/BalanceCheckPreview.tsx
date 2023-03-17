import { Badge, Box, Group, Text } from '@mantine/core';
import BigNumber from 'bignumber.js';
import { format } from 'date-fns';
import { JournalBalanceCheckItem } from '../../rest-model';
import Amount from '../Amount';
import DashLine from '../DashedLine';
import Section from '../Section';

interface Props {
  data: JournalBalanceCheckItem;
}
export default function BalanceCheckPreview(props: Props) {
  const isBalanced = new BigNumber(props.data.postings[0].account_after_number).eq(new BigNumber(props.data.postings[0].account_before_number))
    
  return (
    <div>
      <Box mb={10}>
        <Box mx={1} my={2}>
          {format(new Date(props.data.datetime), 'yyyy-MM-dd hh:mm:ss')}
        </Box>
        {!isBalanced && (
          <Box mx={1} my={2}>
            <Text color={'red'}>UNBALANCED</Text>
          </Box>
        )}
        <Box mx={1} my={2}>
          <Text>{props.data.payee}</Text>
        </Box>
        <Box mx={1} my={2}>
          {props.data.narration}
        </Box>
        <Group mx={1} my={2} spacing="sm">
          {(props.data.links || []).map((link) => (
            <Badge key={link} size="lg" variant="dot">
              {link}
            </Badge>
          ))}
          {(props.data.tags || []).map((tag) => (
            <Badge key={tag} size="lg" color="orange" variant="dot">
              {tag}
            </Badge>
          ))}
        </Group>
      </Box>

      <Box mx={1} my={4}>
        <Section title="Postings">
          <>
            {props.data.postings.map((posting, idx) => (
              <DashLine key={idx}>
                <Text lineClamp={1} my="xs">
                  {posting.account}
                </Text>
                <Text lineClamp={1}><Amount amount={posting.inferred_unit_number} currency={posting.inferred_unit_commodity} /></Text>
              </DashLine>
            ))}
          </>
        </Section>
      </Box>
    </div>
  );
}
