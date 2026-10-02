import { CircleAlert } from 'lucide-react';
import * as React from 'react';
import { useTranslation } from 'react-i18next';
import { OpArgType, OpReturnType, TypedFetch } from 'openapi-typescript-fetch';
import { useAsync } from 'react-use';

interface Props<T> {
  fetcherFunction: TypedFetch<T>;
  params: OpArgType<T>;
  skeleton: React.ReactNode;
  render(data: OpReturnType<T>): React.ReactNode;
  /** Bump to refetch with the same params. */
  reloadKey?: React.Key;
}

/** Fetch with a typed openapi fetcher, showing `skeleton` while loading and an inline error on failure. */
export default function LoadingComponent<T>(props: Props<T>) {
  const { t } = useTranslation();
  const { value: data, error } = useAsync(async () => {
    const res = await props.fetcherFunction(props.params);
    return res.data;
  }, [JSON.stringify(props.params), props.reloadKey]);
  if (error) {
    return (
      <div role="alert" className="flex items-center gap-2 rounded-lg border border-dashed p-4 text-sm text-muted-foreground">
        <CircleAlert className="size-4 text-destructive" aria-hidden />
        {t('ledger.common.load_failed')}
      </div>
    );
  }
  if (!data) return <>{props.skeleton}</>;
  return <>{props.render(data)}</>;
}
