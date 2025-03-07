import * as React from 'react';

import { OpArgType, OpReturnType, TypedFetch } from 'openapi-typescript-fetch';
import { useAsync } from 'react-use';

interface Props<T> {
  fetcherFunction: TypedFetch<T>;
  params: OpArgType<T>;
  skeleton: React.ReactNode;
  render(data: OpReturnType<T>): React.ReactNode;
}

export default function LoadingComponent<T>(props: Props<T>) {
  const { value: data, error } = useAsync(async () => {
    const res = await props.fetcherFunction(props.params);
    return res.data;
  }, [props.params]);
  if (error) return <div>failed to load</div>;
  if (!data) return <>{props.skeleton}</>;
  return <>{props.render(data)}</>;
}
