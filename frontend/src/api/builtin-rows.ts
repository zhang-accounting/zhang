/**
 * A query result's rows by column name: `POST /api/query` and `POST /api/query/builtins/{name}` send each row as a list of
 * cells in the order of `columns`. `Row` is the generated `Builtins[name]['row']` of a built-in query (builtins.ts).
 */
export function rowsByColumn<Row>(columns: readonly { name: string }[], rows: readonly (readonly unknown[])[]): Row[] {
  return rows.map((row) => Object.fromEntries(columns.map((column, index) => [column.name, row[index] ?? null])) as Row);
}
