import * as React from 'react';
import useSWR from 'swr';

import { fetcher } from '../../global.ts';

interface Props<T> {
  url: string;
  skeleton: React.ReactNode;

  render(data: T): React.ReactNode;
}

export default function LoadingComponent<T>(props: Props<T>) {
  const { data, error } = useSWR<T>(props.url, fetcher);
  if (error) return <div>failed to load</div>;
  if (!data) return <>{props.skeleton}</>;
  return <>{props.render(data ?? [])}</>;
}
