import type BigNumber from 'bignumber.js';
import { useAtomValue } from 'jotai';
import { useCallback } from 'react';
import { type AmountTextOptions, formatAmountText } from '@/components/amount-text';
import { loadable_unwrap } from '@/states';
import { commoditiesAtom } from '@/states/commodity';

/**
 * `formatAmountText` with the ledger's commodities: writes an amount as text the way `Amount` shows it (precision, prefix,
 * suffix), for places that need a string, such as a tooltip.
 */
export function useAmountText() {
  const commodities = useAtomValue(commoditiesAtom);
  return useCallback(
    (amount: BigNumber.Value, currency: string, options?: AmountTextOptions) =>
      formatAmountText(
        amount,
        currency,
        loadable_unwrap(commodities, undefined, (all) => all[currency]),
        options,
      ),
    [commodities],
  );
}
