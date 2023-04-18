import { BigNumber } from 'bignumber.js';
import { JournalTransactionItem } from '../rest-model';

export interface SummaryItem {
  number: BigNumber;
  currency: string;
}

interface CurrencyCount {
  [currency: string]: BigNumber;
}

const transfer = (counter: CurrencyCount, amount: BigNumber, currency: string) => {
  if (!counter.hasOwnProperty(currency)) {
    counter[currency] = new BigNumber(0);
  }
  counter[currency] = counter[currency].plus(amount);
};

export function calculate(trx: JournalTransactionItem): Set<SummaryItem> {
  let counter: CurrencyCount = {};
  let internal: CurrencyCount = {};

  trx.postings.forEach((posting) => {
    const unit_number = posting.unit_number || posting.inferred_unit_number;
    const unit_commodity = posting.unit_commodity || posting.inferred_unit_commodity;
    const amount = new BigNumber(unit_number);

    switch (posting.account.split(':')[0].toLocaleLowerCase()) {
      case 'assets':
      case 'liabilities':
        transfer(internal, amount, unit_commodity);
        break;

      case 'expenses':
        transfer(counter, amount, unit_commodity);
        transfer(internal, amount, unit_commodity);
        break;

      case 'income':
        transfer(counter, amount, unit_commodity);
        transfer(internal, amount, unit_commodity);
        break;
      default:
        break;
    }
  });
  const ret = new Set<SummaryItem>();
  Object.keys(counter).forEach((currency) => {
    const targetAmount = counter[currency];
    transfer(internal, targetAmount.negated(), currency);
  });
  Object.keys(internal).forEach((currency) => {
    const targetAmount = internal[currency];
    if (!targetAmount.isZero()) {
      ret.add({ number: targetAmount, currency: currency });
    }
  });
  return ret;
}
