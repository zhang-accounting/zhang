import { atom } from 'jotai';
import { loadable } from 'jotai/utils';
import { retrieveQuerySchema } from '../api/requests';
import { loadable_unwrap } from '.';

/**
 * The tables, columns, functions and keywords of the query language (`/api/query/schema`): those of the served ledger,
 * its providers' tables (such as the budget tables of `zhang:budget`) included. Fetched once, since today every ledger
 * has the same ones.
 * TODO(P1.4): plugins make them the ledger's own, so refetch on a reload (the Reload button and the server's reload events).
 */
export const querySchemaAtom = loadable(atom(async () => (await retrieveQuerySchema({})).data.data));

/** The words the query parser reads as keywords; none while the schema loads. */
export const queryKeywordsAtom = atom((get) => loadable_unwrap(get(querySchemaAtom), [] as string[], (schema) => schema.keywords));
