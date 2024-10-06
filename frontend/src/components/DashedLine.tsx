import { ReactNode } from 'react';


interface Props {
  children: ReactNode;
}

export default function DashLine({ children }: Props) {
  return (
    <div className="flex items-center justify-between py-2 [&+&]:border-t [&+&]:border-dashed [&+&]:border-dark-0 ">
      {children}
    </div>
  );
}
