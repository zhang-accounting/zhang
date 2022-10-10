import { BigNumber } from 'bignumber.js';
import { Amount, TransactionDto } from '../gql/jouralList';

export interface SummaryItem {
  number: BigNumber;
  currency: string;
}

interface CurrencyCount {
  [currency: string]: BigNumber;
}

const transfer = (counter: CurrencyCount, amount: BigNumber, currency: string) => {
  const previousAmount = counter[currency] || new BigNumber(0);
  counter[currency] = previousAmount.plus(amount);
};

export function calculate(trx: TransactionDto): Set<SummaryItem> {
  let counter: CurrencyCount = {};
  let internal: CurrencyCount = {};

  trx.postings.forEach((posting) => {
    const unit = posting.unit || posting.inferredUnit;
    const amount = new BigNumber(unit.number);

    switch (posting.account.accountType) {
      case 'Assets':
      case 'Liabilities':
        transfer(internal, amount, unit.currency);
        break;

      case 'Expenses':
        transfer(counter, amount, unit.currency);
        transfer(internal, amount, unit.currency);
        break;

      case 'Income':
        transfer(counter, amount, unit.currency);
        transfer(internal, amount, unit.currency);
        break;

      default:
        break;
    }
  });
  const ret = new Set<SummaryItem>();
  Object.keys(counter).forEach((currency) => {
    const targetAmount = counter[currency];
    if (!targetAmount.isZero()) {
      ret.add({ number: targetAmount.negated(), currency: currency });
    }
  });
  Object.keys(internal).forEach((currency) => {
    const targetAmount = internal[currency];
    if (!targetAmount.isZero()) {
      ret.add({ number: targetAmount, currency: currency });
    }
  });

  console.log('temp', trx, counter, internal, ret);
  return ret;
}
