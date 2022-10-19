import BigNumber from 'bignumber.js';
import { AccountItem } from '../gql/accountList';
import { TransactionDto } from '../gql/jouralList';
import { calculate } from './trx-calculator';

export {};

function simpleTrx(postings: any): TransactionDto {
  return {
    date: '',
    timestamp: 0,
    type: 'TransactionDto',
    payee: '',
    postings,
    tags: [],
    links: [],
    metas: [],
    isBalanced: false,
    spanEnd: 0,
    spanFile: '',
  };
}

describe('transaction summary calculator', () => {
  it('should calculate expense given same currency', () => {
    const trx: TransactionDto = simpleTrx([
      {
        account: {
          name: 'Assets:12332',
          status: 'OPEN',
          accountType: 'Assets',
        } as AccountItem,
        unit: {
          number: '-100',
          currency: 'CNY',
        },
        infer_unit: {
          number: '-100',
          currency: 'CNY',
        },
      },
      {
        account: {
          name: 'Expenses:12332',
          status: 'OPEN',
          accountType: 'Expenses',
        } as AccountItem,
        unit: {
          number: '100',
          currency: 'CNY',
        },
        infer_unit: {
          number: '100',
          currency: 'CNY',
        },
      },
    ]);
    expect(calculate(trx)).toEqual(
      new Set([
        {
          number: new BigNumber('-100'),
          currency: 'CNY',
        },
      ]),
    );
  });

  it('should calculate income given same currency', () => {
    const trx: TransactionDto = simpleTrx([
      {
        account: {
          name: 'Assets:12332',
          status: 'OPEN',
          accountType: 'Assets',
        } as AccountItem,
        unit: {
          number: '100',
          currency: 'CNY',
        },
        infer_unit: {
          number: '100',
          currency: 'CNY',
        },
      },
      {
        account: {
          name: 'Income:12332',
          status: 'OPEN',
          accountType: 'Income',
        } as AccountItem,
        unit: {
          number: '-100',
          currency: 'CNY',
        },
        infer_unit: {
          number: '-100',
          currency: 'CNY',
        },
      },
    ]);
    expect(calculate(trx)).toEqual(
      new Set([
        {
          number: new BigNumber('100'),
          currency: 'CNY',
        },
      ]),
    );
  });
  it('should return empty given internal transaction', () => {
    const trx: TransactionDto = simpleTrx([
      {
        account: {
          name: 'Assets:12332',
          status: 'OPEN',
          accountType: 'Assets',
        } as AccountItem,
        unit: {
          number: '100',
          currency: 'CNY',
        },
        infer_unit: {
          number: '100',
          currency: 'CNY',
        },
      },
      {
        account: {
          name: 'Assets:12332',
          status: 'OPEN',
          accountType: 'Assets',
        } as AccountItem,
        unit: {
          number: '-100',
          currency: 'CNY',
        },
        infer_unit: {
          number: '-100',
          currency: 'CNY',
        },
      },
    ]);
    expect(calculate(trx)).toEqual(new Set([]));
  });
  it('should return currency info given internal transaction with diff currency', () => {
    const trx: TransactionDto = simpleTrx([
      {
        account: {
          name: 'Assets:12332',
          status: 'OPEN',
          accountType: 'Assets',
        } as AccountItem,
        unit: {
          number: '1',
          currency: 'CNY100',
        },
        infer_unit: {
          number: '100',
          currency: 'CNY',
        },
      },
      {
        account: {
          name: 'Assets:12332',
          status: 'OPEN',
          accountType: 'Assets',
        } as AccountItem,
        unit: {
          number: '-100',
          currency: 'CNY',
        },
        infer_unit: {
          number: '-100',
          currency: 'CNY',
        },
      },
    ]);
    expect(calculate(trx)).toEqual(
      new Set([
        {
          number: new BigNumber('1'),
          currency: 'CNY100',
        },
        {
          number: new BigNumber('-100'),
          currency: 'CNY',
        },
      ]),
    );
  });
});
