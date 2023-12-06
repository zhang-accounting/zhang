import { Box, Space } from '@mantine/core';
import { ReactElement } from 'react';
import { BudgetListItem } from '../../rest-model';
import Amount from '../Amount';
interface Props extends BudgetListItem {}
export default function BudgetLine(props: Props) {
  return (
    <tr>
      <td>
        <div style={{ display: 'flex' }}>
          <Space w={22}></Space>
          {props.alias ?? props.name}
        </div>
      </td>
      <td style={{ textAlign: 'end' }}>
        <Amount amount={props.assigned_amount.number} currency={props.assigned_amount.currency} />
      </td>
      <td style={{ textAlign: 'end' }}>
        <Amount amount={props.activity_amount.number} currency={props.activity_amount.currency} />
      </td>
      <td style={{ textAlign: 'end' }}>
        <Amount amount={props.available_amount.number} currency={props.available_amount.currency} />
      </td>
    </tr>
  );
}
