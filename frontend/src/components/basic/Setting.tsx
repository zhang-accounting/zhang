import { ReactElement } from 'react';

interface Props {
  title: string;
  value?: string;
  uppercase?: boolean;
  children?: ReactElement | ReactElement[];
}

export function Setting({ title, value, children, uppercase }: Props) {
  const shouldUppercase = uppercase ?? false;
  return (
    <div>
      <p className="text-gray-500 text-sm font-bold">{shouldUppercase ? title.toUpperCase() : title}</p>
      {value && <p>{value}</p>}
      {children}
    </div>
  );
}
