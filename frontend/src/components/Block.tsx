import { ReactElement } from 'react';
interface Props {
  title?: string;
  children: ReactElement | ReactElement[];
}
export default function Block({ title, children }: Props) {
  return (
    <div className="px-4 py-2">
      {title && <div className="mb-2">{title}</div>}
      <div>{children}</div>
    </div>
  );
}
