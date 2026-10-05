import type BigNumber from 'bignumber.js';
import { useAtomValue } from 'jotai';
import { useCallback } from 'react';
import { type AmountTextOptions, formatAmountText } from '@/components/amount-text';
import { loadable_unwrap } from '@/states';
import { commoditiesAtom } from '@/states/commodity';
import { defaultCommodityPrecisionAtom } from '@/states/options';

/**
 * `formatAmountText` with the ledger's commodities and its `default_commodity_precision`: writes an amount as text the way
 * `Amount` shows it (precision, prefix, suffix), for places that need a string, such as a tooltip.
 */
export function useAmountText() {
  const commodities = useAtomValue(commoditiesAtom);
  const defaultPrecision = useAtomValue(defaultCommodityPrecisionAtom);
  return useCallback(
    (amount: BigNumber.Value, currency: string, options?: AmountTextOptions) =>
      formatAmountText(
        amount,
        currency,
        loadable_unwrap(commodities, undefined, (all) => all[currency]),
        { defaultPrecision, ...options },
      ),
    [commodities, defaultPrecision],
  );
}
