import { atom } from 'jotai';
import { atomWithRefresh, loadable } from 'jotai/utils';
import { optionValue, retrieveOptions } from '../api/requests';
import { DEFAULT_COMMODITY_PRECISION, defaultPrecisionOption } from '../components/amount-text';
import { loadable_unwrap } from '.';

/**
 * The ledger's options (`/api/options`), the defaults it did not set included: fetched once for every page that shows or
 * uses one, and refreshed when the ledger reloads (SSE `Reload`).
 */
export const optionsFetcher = atomWithRefresh(async () => {
  return (await retrieveOptions({})).data.data;
});

export const optionsAtom = loadable(optionsFetcher);

/** The ledger's operating currency; `undefined` while the options load, or when it is not set. */
export const operatingCurrencyAtom = atom((get) => {
  return loadable_unwrap(get(optionsAtom), undefined, (options) => optionValue(options, 'operating_currency'));
});

/** The ledger's `default_commodity_precision`, the precision of a commodity it does not declare; the built-in default while the options load. */
export const defaultCommodityPrecisionAtom = atom((get) => {
  return loadable_unwrap(get(optionsAtom), DEFAULT_COMMODITY_PRECISION, (options) =>
    defaultPrecisionOption(optionValue(options, 'default_commodity_precision')),
  );
});
