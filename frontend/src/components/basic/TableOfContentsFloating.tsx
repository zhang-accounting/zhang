import { NavLink } from '@mantine/core';
import { IconFileInvoice, IconFolder } from '@tabler/icons-react';

export const ZHANG_VALUE = Symbol();

export interface Tier {
  [ZHANG_VALUE]?: string;

  [key: string]: Tier;
}

interface TableOfContentsFloatingProps {
  files: Tier;

  onChange(value: string): void;
}

export function TableOfContentsFloating({ files, onChange }: TableOfContentsFloatingProps) {
  const itemBuilder = (item: Tier, index: number) => {
    return Object.keys(item).map((key) => {
      const targetItem = item[key];
      const isFile = !!targetItem[ZHANG_VALUE];
      return (
        <>
          <NavLink
            label={key}
            leftSection={isFile ? <IconFileInvoice size={16} stroke={1.5} /> : <IconFolder size={16} stroke={1.5} />}
            childrenOffset={14}
            onClick={() => {
              if (isFile) {
                onChange(targetItem[ZHANG_VALUE]!);
              }
            }}
          >
            {!isFile && itemBuilder(targetItem, index + 1)}
          </NavLink>
        </>
      );
    });
  };
  const items = itemBuilder(files, 1);

  return <>{items}</>;
}
