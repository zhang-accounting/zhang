import { ReactNode } from 'react';


interface Props {
  children: ReactNode;
}

export default function DashLine({ children }: Props) {
  return (
    <div className="flex item-center border-t border-dashed border-dark-0 my-1 [&+&]:border-t [&+&]:border-dashed [&+&]:border-dark-0 [&+&]:my-1  justify-between">
      {children}
    </div>
  );
}
