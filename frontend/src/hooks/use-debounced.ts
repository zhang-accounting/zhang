import { useCallback, useEffect, useRef, useState } from 'react';

/** Returns `[debounced]`: `value` delayed by `wait` ms. */
export function useDebouncedValue<T>(value: T, wait: number) {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timeout = window.setTimeout(() => setDebounced(value), wait);
    return () => window.clearTimeout(timeout);
  }, [value, wait]);
  return [debounced] as const;
}

/** State whose setter is debounced by `wait` ms: `const [value, setValue] = useDebouncedState('', 200)`. */
export function useDebouncedState<T>(defaultValue: T, wait: number) {
  const [value, setValue] = useState(defaultValue);
  const timeoutRef = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timeoutRef.current), []);
  const debouncedSetValue = useCallback(
    (next: T) => {
      window.clearTimeout(timeoutRef.current);
      timeoutRef.current = window.setTimeout(() => setValue(next), wait);
    },
    [wait],
  );
  return [value, debouncedSetValue] as const;
}
