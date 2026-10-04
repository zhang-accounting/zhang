import { useCallback, useEffect, useState } from 'react';

/**
 * JSON-serialised localStorage state, kept in sync between every hook instance using the same key
 * (same tab via a custom event, other tabs via the `storage` event).
 * Signature kept from the previous hooks library: `useLocalStorage({ key, defaultValue })`.
 */
interface UseLocalStorageOptions<T> {
  key: string;
  defaultValue: T;
}

const LOCAL_STORAGE_EVENT = 'zhang:local-storage';

function read<T>(key: string, defaultValue: T): T {
  if (typeof window === 'undefined') return defaultValue;
  try {
    const raw = window.localStorage.getItem(key);
    if (raw === null) return defaultValue;
    try {
      return JSON.parse(raw) as T;
    } catch {
      return raw as unknown as T;
    }
  } catch {
    return defaultValue;
  }
}

/** Stores `value` under `key` and notifies every `useLocalStorage` hook of that key, in this tab and in the others. */
export function writeLocalStorage<T>(key: string, value: T) {
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // storage full or unavailable: keep in-memory state only
  }
  window.dispatchEvent(new CustomEvent(LOCAL_STORAGE_EVENT, { detail: key }));
}

export function useLocalStorage<T>({ key, defaultValue }: UseLocalStorageOptions<T>) {
  const [value, setValue] = useState<T>(() => read(key, defaultValue));

  useEffect(() => {
    setValue(read(key, defaultValue));
    const onChange = (event: Event) => {
      if (event instanceof StorageEvent) {
        if (event.key !== key) return;
      } else if ((event as CustomEvent<string>).detail !== key) {
        return;
      }
      setValue(read(key, defaultValue));
    };
    window.addEventListener('storage', onChange);
    window.addEventListener(LOCAL_STORAGE_EVENT, onChange);
    return () => {
      window.removeEventListener('storage', onChange);
      window.removeEventListener(LOCAL_STORAGE_EVENT, onChange);
    };
    // defaultValue is intentionally excluded: callers usually pass a new literal each render
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  const setStoredValue = useCallback(
    (next: T | ((prev: T) => T)) => {
      const resolved = typeof next === 'function' ? (next as (prev: T) => T)(read(key, defaultValue)) : next;
      setValue(resolved);
      writeLocalStorage(key, resolved);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [key],
  );

  return [value, setStoredValue] as const;
}
