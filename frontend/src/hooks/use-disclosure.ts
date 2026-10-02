import { useCallback, useMemo, useState } from 'react';

/** Boolean open/close state: `const [opened, { open, close, toggle }] = useDisclosure()`. */
export function useDisclosure(initialState = false) {
  const [opened, setOpened] = useState(initialState);
  const open = useCallback(() => setOpened(true), []);
  const close = useCallback(() => setOpened(false), []);
  const toggle = useCallback(() => setOpened((current) => !current), []);
  const handlers = useMemo(() => ({ open, close, toggle, set: setOpened }), [open, close, toggle]);
  return [opened, handlers] as const;
}
