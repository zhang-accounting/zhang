import { Group, Space } from '@mantine/core';
import React from 'react';
import { BudgetListItem } from '../../rest-model';
import Amount from '../Amount';
import BigNumber from 'bignumber.js';
import BackgroundProgress from '../basic/BackgroundProgress';
import { useNavigate } from 'react-router';

interface Props extends BudgetListItem {}

export default function BudgetLine(props: Props) {
  const navigate = useNavigate();
  let number = new BigNumber(props.activity_amount.number).div(new BigNumber(props.assigned_amount.number)).multipliedBy(100).toPrecision(2);
  return (
    <tr style={{ position: 'relative', zIndex: 1 }}>
      <td>
        <BackgroundProgress percentage={number} />
        <Group>
          <Space w={6}></Space>
          <div style={{ cursor: 'pointer' }} onClick={() => navigate(props.name)}>
            {props.alias ?? props.name}
          </div>
        </Group>
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
