import { Badge, Box, Text } from '@mantine/core';
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
  const isBalanced = new BigNumber(props.data.postings[0].account_after_number).eq(new BigNumber(props.data.postings[0].account_before_number));
  const checkInfo = props.data.postings[0];
  return (
    <div>
      <Box mb={10}>
        <Section title="Check Info">
          <DashLine>
            <Text lineClamp={1} my="xs">
              Datetime
            </Text>
            <Text lineClamp={1}>{format(new Date(props.data.datetime), 'yyyy-MM-dd HH:mm:ss')}</Text>
          </DashLine>
          <DashLine>
            <Text lineClamp={1} my="xs">
              Account
            </Text>
            <Text lineClamp={1}>{checkInfo.account}</Text>
          </DashLine>
          <DashLine>
            <Text lineClamp={1} my="xs">
              Check Status
            </Text>
            <Text lineClamp={1}>
              {isBalanced ? (
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
              Balance Amount
            </Text>
            <Text lineClamp={1}>
              <Amount amount={checkInfo.account_after_number} currency={checkInfo.account_after_commodity} />
            </Text>
          </DashLine>

          {!isBalanced && (
            <>
              <DashLine>
                <Text lineClamp={1} my="xs">
                  Accumulated Amount
                </Text>
                <Text lineClamp={1}>
                  <Amount amount={checkInfo.account_before_number} currency={checkInfo.account_before_commodity} />
                </Text>
              </DashLine>

              <DashLine>
                <Text lineClamp={1} my="xs">
                  Distance
                </Text>
                <Text lineClamp={1}>
                  <Amount amount={checkInfo.inferred_unit_number} currency={checkInfo.inferred_unit_commodity} />
                </Text>
              </DashLine>
            </>
          )}
        </Section>
      </Box>
    </div>
  );
}
