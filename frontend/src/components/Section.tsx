import * as React from 'react';
import { Card, CardContent, CardHeader, CardTitle } from './ui/card';

interface Props {
  title: string;
  rightSection?: React.ReactNode;
  children: React.ReactNode;
  noPadding?: boolean;
}

export default function Section({ children, title, rightSection }: Props) {
  return (
    <Card className="mt-2 rounded-sm border-2 border-gray-100 bg-transparent hover:border-primary">
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2 ">
        <CardTitle className="text-sm font-medium text-gray-900">{title}</CardTitle>
        {rightSection}
      </CardHeader>
      <CardContent>
        {children}
      </CardContent>
    </Card>
  );
}
