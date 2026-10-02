import { ReactElement } from 'react';

interface Props {
  title?: string;
  children: ReactElement | ReactElement[];
}

export default function Block({ title, children }: Props) {
  return (
    <div className="flex flex-col gap-2 px-4 py-2">
      {title && <div className="text-xs font-medium text-muted-foreground">{title}</div>}
      <div>{children}</div>
    </div>
  );
}
