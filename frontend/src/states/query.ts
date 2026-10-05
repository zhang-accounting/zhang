import { atom } from 'jotai';
import { loadable } from 'jotai/utils';
import { retrieveQuerySchema } from '../api/requests';
import { loadable_unwrap } from '.';

/** The tables, columns, functions and keywords of the query language (`/api/query/schema`), fetched once: they are the engine's. */
export const querySchemaAtom = loadable(atom(async () => (await retrieveQuerySchema({})).data.data));

/** The words the query parser reads as keywords; none while the schema loads. */
export const queryKeywordsAtom = atom((get) => loadable_unwrap(get(querySchemaAtom), [] as string[], (schema) => schema.keywords));
