/** One group of an account picker: the accounts of one type, such as `Assets` or `Expenses`. */
export interface AccountOptionGroup {
  group: string;
  items: { value: string; label: string }[];
}

/**
 * The options of an account picker, grouped by account type and sorted by name: the accounts the server lists as open
 * (`journals.accounts`, by the rule the ledger checks its directives with), and the accounts in `keep`,
 * such as those the postings of an edited transaction already use, so a value already chosen still shows when its
 * account is not open.
 */
export function accountOptions(open: readonly string[], keep: readonly (string | undefined)[] = []): AccountOptionGroup[] {
  const names = [...new Set([...open, ...keep.filter((it): it is string => !!it)])].sort();
  const groups = new Map<string, string[]>();
  for (const name of names) {
    const group = name.split(':')[0];
    groups.set(group, [...(groups.get(group) ?? []), name]);
  }
  return [...groups.keys()].sort().map((group) => ({
    group,
    items: (groups.get(group) ?? []).map((it) => ({ value: it, label: it })),
  }));
}
