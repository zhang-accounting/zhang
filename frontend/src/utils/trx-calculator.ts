import { JournalTransactionItem } from '@/api/types';
import { BigNumber } from 'bignumber.js';

export interface SummaryItem {
  number: BigNumber;
  commodity: string;
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
    const unit_number = posting.unit?.number || posting.inferred_unit.number;
    const unit_commodity = posting.unit?.commodity || posting.inferred_unit.commodity;
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
      ret.add({ number: targetAmount, commodity: currency });
    }
  });
  return ret;
}
