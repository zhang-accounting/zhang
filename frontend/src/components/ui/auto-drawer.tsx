import * as React from 'react';
import { useMediaQuery } from '@mantine/hooks';
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

export function AutoDrawer({
  open,
  onOpenChange,
  children,
  title,
  description,
  footer,
  className,
}: AutoDrawerProps) {
  const isMobile = useMediaQuery('(max-width: 640px)');
  const childrenArray = React.Children.toArray(children);
  const triggerIndex = childrenArray.findIndex(
    (child) => React.isValidElement(child) && child.type === AutoDrawerTrigger
  );
  const trigger = triggerIndex !== -1 ? childrenArray[triggerIndex] : null;
  const otherChildren = childrenArray.filter((_, index) => index !== triggerIndex);

  if (isMobile) {
    return (
      <>
        <Drawer open={open} onOpenChange={onOpenChange}>
          {trigger}
          <DrawerContent className={cn("px-4", className)}>
            {(title || description) && (
            <DrawerHeader>
              {title && <DrawerTitle>{title}</DrawerTitle>}
              {description && <DrawerDescription>{description}</DrawerDescription>}
            </DrawerHeader>
          )}
          {otherChildren}
          {footer && <DrawerFooter>{footer}</DrawerFooter>}
        </DrawerContent>
      </Drawer>
      </>
    );
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      {trigger}
      <DialogContent className={cn("sm:max-w-[425px]", className)}>
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

interface AutoDrawerTriggerProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  asChild?: boolean;
}

export const AutoDrawerTrigger = React.forwardRef<HTMLButtonElement, AutoDrawerTriggerProps>(
  ({ asChild, ...props }, ref) => {
    const isMobile = useMediaQuery('(max-width: 640px)');
    const Component = isMobile ? DrawerTrigger : DialogTrigger;
    return <Component ref={ref} asChild={asChild} {...props} />;
  }
); 