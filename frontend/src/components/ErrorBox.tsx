import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { errorAtom, errorPageAtom, LedgerError } from '../states/errors';
import { ErrorsSkeleton } from './skeletons/errorsSkeleton';
import { useAtomValue, useSetAtom } from 'jotai';
import Joyride from '../assets/joyride.svg';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from './ui/dialog';
import { Button } from './ui/button';
import { Textarea } from './ui/textarea';
import { useDisclosure } from '@mantine/hooks';
import { Pagination, PaginationContent, PaginationItem, PaginationLink, PaginationNext, PaginationPrevious } from './ui/pagination';

export default function ErrorBox() {
  const { t } = useTranslation();
  const [isOpen, isOpenHandler] = useDisclosure(false);

  const [selectError, setSelectError] = useState<LedgerError | null>(null);
  const [selectErrorContent, setSelectErrorContent] = useState<string>('');

  const errors = useAtomValue(errorAtom);
  const setErrorPage = useSetAtom(errorPageAtom);

  if (errors.state === 'loading' || errors.state === 'hasError') {
    return <ErrorsSkeleton />;
  }
  const handlePageChange = (newPage: number) => {
    setErrorPage(newPage);
  };

  const toggleError = (error: LedgerError) => {
    setSelectError(error);
    setSelectErrorContent(error.span.content);
    isOpenHandler.open();
  };

  const saveErrorModifyData = () => {
    //   modifyFile({
    //     variables: {
    //       file: selectError?.span.filename,
    //       content: selectErrorContent,
    //       start: selectError?.span.start,
    //       end: selectError?.span.end,
    //     },
    //   });
    isOpenHandler.close();
  };
  const onModalReset = () => {
    setSelectErrorContent(selectError?.span.content || '');
  };
  return (
    <>
      <Dialog open={isOpen} onOpenChange={() => isOpenHandler.close()}>
        <DialogContent className='max-h-[90vh] w-2/3 overflow-y-auto'>
          <DialogHeader>
            <DialogTitle>{`${selectError?.span.filename}:L${selectError?.span.start}:${selectError?.span.end}`}</DialogTitle>
            <DialogDescription>
              <p>{t(`ERROR.${selectError?.error_type || ''}`)}</p>
              <Textarea
                rows={selectErrorContent.split('\n').length}
                value={selectErrorContent}
                onChange={(event) => {
                  setSelectErrorContent(event.target.value);
                }}
              />
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button onClick={onModalReset} variant="outline">
              {t('RESET')}
            </Button>
            <Button onClick={saveErrorModifyData} variant="default">
              {t('SAVE')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <div className="flex flex-col gap-2">
        {errors.data.total_count === 0 ? (
          <div className="flex flex-col items-center">
            <img className='w-1/2 rounded-md' src={Joyride} />
            <p className='text-xl font-bold'>{t('LEDGER_IS_HEALTHY')}</p>
          </div>
        ) : (
          <>
            {errors.data.records.map((error, idx) => (
              <p key={idx} className='cursor-pointer' onClick={() => toggleError(error)}>
                {t(`ERROR.${error.error_type}`)}
              </p>
            ))}

            <Pagination>
              <PaginationContent>
                {errors.data.current_page > 1 && (
                  <PaginationItem>
                    <PaginationPrevious href="#" onClick={() => handlePageChange(errors.data.current_page - 1)} />
                  </PaginationItem>
                )}
                {errors.data.current_page > 1 && (
                  <PaginationItem>
                    <PaginationLink href="#">
                      {errors.data.current_page - 1}
                    </PaginationLink>
                  </PaginationItem>
                )}
                <PaginationItem>
                  <PaginationLink href="#" isActive>
                    {errors.data.current_page}
                  </PaginationLink>
                </PaginationItem>
                {errors.data.current_page < errors.data.total_page && (
                  <PaginationItem>
                    <PaginationLink href="#">{errors.data.current_page + 1}</PaginationLink>
                  </PaginationItem>
                )}
                {errors.data.current_page < errors.data.total_page && (
                  <PaginationItem>
                    <PaginationNext href="#" onClick={() => handlePageChange(errors.data.current_page + 1)} />
                  </PaginationItem>
                )}
              </PaginationContent>
            </Pagination>
          </>
        )}
      </div>
    </>
  );
}
