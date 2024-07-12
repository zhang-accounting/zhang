import { Loadable } from 'jotai/vanilla/utils/loadable';

export function loadable_unwrap<T, F>(val: Loadable<Promise<T>>, init_value: F, mapper: (data: T) => F): F {
  if (val.state === 'hasError' || val.state === 'loading') {
    return init_value;
  } else {
    return mapper(val.data as T);
  }
}
