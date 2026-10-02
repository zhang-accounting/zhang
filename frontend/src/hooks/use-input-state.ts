import { ChangeEvent, useCallback, useState } from 'react';

/** State whose setter accepts either a value or an input change event. */
export function useInputState<T = string>(initialState: T) {
  const [value, setValue] = useState<T>(initialState);
  const onChange = useCallback((next: T | ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    if (next !== null && typeof next === 'object' && 'currentTarget' in (next as object)) {
      const target = (next as ChangeEvent<HTMLInputElement>).currentTarget;
      setValue((target.type === 'checkbox' ? target.checked : target.value) as T);
    } else {
      setValue(next as T);
    }
  }, []);
  return [value, onChange] as const;
}
