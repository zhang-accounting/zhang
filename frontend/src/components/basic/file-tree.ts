export const ZHANG_VALUE = Symbol();

export interface Tier {
  [ZHANG_VALUE]?: string;

  [key: string]: Tier;
}

/** Builds a nested folder tree from ledger file paths (`data/2023/12.zhang`). Leaves carry the full path in `ZHANG_VALUE`. */
export function buildFileTree(paths: (string | null)[]): Tier {
  const tree: Tier = {};
  paths
    .filter((it): it is string => it !== null)
    .map((it) => it.replace(/^\/|\/$/g, ''))
    .forEach((entry) => {
      let node = tree;
      entry.split(/[/\\]/).forEach((segment) => {
        if (!Object.prototype.hasOwnProperty.call(node, segment)) node[segment] = {};
        node = node[segment];
      });
      node[ZHANG_VALUE] = entry;
    });
  return tree;
}
