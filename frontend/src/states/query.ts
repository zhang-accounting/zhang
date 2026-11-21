import { atom } from 'jotai';
import { loadable, unwrap } from 'jotai/utils';
import { retrieveQuerySchema } from '../api/requests';
import { loadable_unwrap } from '.';
import { ledgerRevisionAtom } from './ledger';

/**
 * The tables, columns, functions and keywords of the query language (`/api/query/schema`) of the served ledger: zhang's
 * own, and the tables its plugins provide (such as the budget tables of `zhang:budget`, or an analyzer's). A ledger's
 * plugins can change with its files, so the schema is fetched again whenever the ledger may have changed (the Reload
 * button, the server's reload events, a write in the app).
 */
const fetchedSchemaAtom = atom(async (get) => {
  get(ledgerRevisionAtom);
  return (await retrieveQuerySchema({})).data.data;
});

const schemaStatusAtom = loadable(fetchedSchemaAtom);

/** The schema fetched last, which stays while a newer one loads; undefined before the first. */
const latestSchemaAtom = unwrap(fetchedSchemaAtom, (previous) => previous);

/** The schema as a loadable: loading only before the first one arrives, the previous one meanwhile after that. */
export const querySchemaAtom = atom((get) => {
  const status = get(schemaStatusAtom);
  if (status.state === 'hasError') return status;
  const latest = get(latestSchemaAtom);
  return latest === undefined ? status : { state: 'hasData' as const, data: latest };
});

/** The words the query parser reads as keywords; none while the schema loads. */
export const queryKeywordsAtom = atom((get) => loadable_unwrap(get(querySchemaAtom), [] as string[], (schema) => schema.keywords));
