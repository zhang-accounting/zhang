import { useMemo, useState } from 'react';

/** Array state with immutable helpers: `const [items, { append, remove, setItemProp, setState }] = useListState(initial)`. */
export function useListState<T>(initialValue: T[] = []) {
  const [state, setState] = useState<T[]>(initialValue);

  const handlers = useMemo(
    () => ({
      setState,
      append: (...items: T[]) => setState((current) => [...current, ...items]),
      remove: (...indices: number[]) => setState((current) => current.filter((_, index) => !indices.includes(index))),
      setItem: (index: number, item: T) => setState((current) => current.map((it, i) => (i === index ? item : it))),
      setItemProp: <K extends keyof T>(index: number, prop: K, value: T[K]) =>
        setState((current) => current.map((it, i) => (i === index ? { ...it, [prop]: value } : it))),
    }),
    [],
  );

  return [state, handlers] as const;
}
