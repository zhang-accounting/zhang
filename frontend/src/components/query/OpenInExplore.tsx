import { SearchCode } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router';
import { toast } from 'sonner';
import { retrieveBuiltinQueryText } from '@/api/requests';
import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import { apiErrorMessage } from '@/lib/api-error';
import { cn } from '@/lib/utils';
import { type BuiltinParamInput, exploreUrl, toBuiltinParams } from './explore-link';

interface OpenInExploreProps {
  /** The built-in query behind the figure, as `GET /api/query/builtins` names it, e.g. `report.summary`. */
  name: string;
  /** The values the page ran the query with, by parameter name; a `Date` is sent as the day it shows. */
  params?: Record<string, BuiltinParamInput>;
  /** Only the icon, with the label as its accessible name and tooltip (e.g. in a card header). */
  iconOnly?: boolean;
  className?: string;
}

/**
 * "Open in Explore": opens the Query page with the built-in query behind a figure in the editor, its parameters filled in
 * with the values the page used, and runs it there.
 */
export function OpenInExplore({ name, params = {}, iconOnly = false, className }: OpenInExploreProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [opening, setOpening] = useState(false);

  const open = async () => {
    setOpening(true);
    try {
      const res = await retrieveBuiltinQueryText({ name, params: toBuiltinParams(params) });
      navigate(exploreUrl(res.data.data.query));
    } catch (error) {
      toast.error(t('query.open_query_failed'), { description: await apiErrorMessage(error) });
    } finally {
      setOpening(false);
    }
  };

  const label = t('query.open_query');
  return (
    <Button
      variant="ghost"
      size={iconOnly ? 'icon-sm' : 'sm'}
      className={cn('text-muted-foreground', className)}
      onClick={open}
      disabled={opening}
      title={t('query.open_query_hint')}
      aria-label={iconOnly ? label : undefined}
    >
      {opening ? <Spinner aria-hidden /> : <SearchCode aria-hidden />}
      {!iconOnly && label}
    </Button>
  );
}
