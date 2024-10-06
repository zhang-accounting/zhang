import { ReactNode } from 'react';
import { Button } from '../ui/button';

interface Props {
  value: String;
  icon?: ReactNode;
  onActionClick?(): void;
}
export default function DividerWithAction({ value, icon, onActionClick }: Props) {
  return (
    <div className="flex items-center justify-between my-2">
      <div className="flex-grow">
        <div className="flex items-center">
          <span className="text-xs font-medium text-gray-500 mr-2">{value}</span>
          <div className="flex-grow h-px bg-gray-200"></div>
        </div>
      </div>
      {icon && (
        <Button variant="ghost" size="icon" className="mx-1" onClick={onActionClick}>
          {icon}
        </Button>
      )}
    </div>
  );
}
