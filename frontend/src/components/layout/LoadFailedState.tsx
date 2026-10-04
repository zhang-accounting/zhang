import * as React from 'react';
import { TriangleAlert } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { EmptyState } from './EmptyState';

export interface LoadFailedStateProps {
  /** Usually the error message. */
  description?: React.ReactNode;
  onRetry: () => void;
}

/** "Failed to load" placeholder with a retry button, shown in place of a page's content when its data request fails. */
export function LoadFailedState({ description, onRetry }: LoadFailedStateProps) {
  const { t } = useTranslation();
  return (
    <EmptyState
      icon={TriangleAlert}
      title={t('page_state.load_failed')}
      description={description}
      action={
        <Button variant="outline" className="h-10 md:h-8" onClick={() => onRetry()}>
          {t('page_state.retry')}
        </Button>
      }
    />
  );
}
