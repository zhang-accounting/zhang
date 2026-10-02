// Custom (not from the shadcn registry): renders a Dialog on >= md and a bottom Drawer on mobile.
import * as React from 'react';
import { useIsMobile } from '@/hooks/use-mobile';
import { Dialog, DialogContent, DialogHeader, DialogFooter, DialogTitle, DialogDescription, DialogTrigger } from './dialog';
import { Drawer, DrawerContent, DrawerHeader, DrawerFooter, DrawerTitle, DrawerDescription, DrawerTrigger } from './drawer';
import { cn } from '@/lib/utils';

interface AutoDrawerProps {
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  children?: React.ReactNode;
  title?: React.ReactNode;
  description?: React.ReactNode;
  footer?: React.ReactNode;
  className?: string;
}

export function AutoDrawer({ open, onOpenChange, children, title, description, footer, className }: AutoDrawerProps) {
  const isMobile = useIsMobile();
  const childrenArray = React.Children.toArray(children);
  const triggerIndex = childrenArray.findIndex((child) => React.isValidElement(child) && child.type === AutoDrawerTrigger);
  const trigger = triggerIndex !== -1 ? childrenArray[triggerIndex] : null;
  const otherChildren = childrenArray.filter((_, index) => index !== triggerIndex);

  if (isMobile) {
    return (
      <Drawer open={open} onOpenChange={(next) => onOpenChange?.(next)}>
        {trigger}
        <DrawerContent className={cn('px-4', className)}>
          {(title || description) && (
            <DrawerHeader>
              {title && <DrawerTitle>{title}</DrawerTitle>}
              {description && <DrawerDescription>{description}</DrawerDescription>}
            </DrawerHeader>
          )}
          <div className="max-h-[70vh] overflow-y-auto overscroll-contain">{otherChildren}</div>
          {footer && <DrawerFooter>{footer}</DrawerFooter>}
        </DrawerContent>
      </Drawer>
    );
  }

  return (
    <Dialog open={open} onOpenChange={(next) => onOpenChange?.(next)}>
      {trigger}
      <DialogContent className={cn('sm:max-w-[425px]', className)}>
        {(title || description) && (
          <DialogHeader>
            {title && <DialogTitle>{title}</DialogTitle>}
            {description && <DialogDescription>{description}</DialogDescription>}
          </DialogHeader>
        )}
        {otherChildren}
        {footer && <DialogFooter>{footer}</DialogFooter>}
      </DialogContent>
    </Dialog>
  );
}

/** Trigger for AutoDrawer. Use Base UI composition: `<AutoDrawerTrigger render={<Button />}>Label</AutoDrawerTrigger>`. */
export function AutoDrawerTrigger(props: React.ComponentProps<typeof DialogTrigger>) {
  const isMobile = useIsMobile();
  return isMobile ? <DrawerTrigger {...(props as React.ComponentProps<typeof DrawerTrigger>)} /> : <DialogTrigger {...props} />;
}
