import { useEffect, useState } from 'react';

/** Returns `[debounced]`: `value` delayed by `wait` ms. */
export function useDebouncedValue<T>(value: T, wait: number) {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timeout = window.setTimeout(() => setDebounced(value), wait);
    return () => window.clearTimeout(timeout);
  }, [value, wait]);
  return [debounced] as const;
}
