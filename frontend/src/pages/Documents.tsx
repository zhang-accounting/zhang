import { retrieveDocuments } from '@/api/requests.ts';
import { Badge } from '@/components/ui/badge.tsx';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table.tsx';
import { DOCUMENTS_LINK } from '@/layout/Sidebar.tsx';
import { useDocumentTitle, useLocalStorage } from '@mantine/hooks';
import { format } from 'date-fns';
import { useAtomValue, useSetAtom } from 'jotai/index';
import { groupBy, reverse, sortBy } from 'lodash-es';
import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router';
import { useAsync } from 'react-use';
import 'yet-another-react-lightbox/styles.css';
import AccountDocumentLine from '../components/documentLines/AccountDocumentLine';
import { ImageLightBox } from '../components/ImageLightBox';
import { breadcrumbAtom, titleAtom } from '../states/basic';
import { isDocumentAnImage } from '../utils/documents';

export default function Documents() {
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  let navigate = useNavigate();
  const [layout, setLayout] = useLocalStorage({
    key: `document-list-layout`,
    defaultValue: 'Grid',
  });
  const { loading, error, value: documents } = useAsync(async () => {
    const res = await retrieveDocuments({});
    return res.data.data;
  }, []);

  const [lightboxSrc, setLightboxSrc] = useState<string | undefined>(undefined);

  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`Documents - ${ledgerTitle}`);
  useEffect(() => {
    setBreadcrumb([DOCUMENTS_LINK]);
  }, []);
  if (error) return <div>failed to load</div>;
  if (loading || !documents) return <div>loading...</div>;

  const groupedDocuments = reverse(
    sortBy(
      groupBy(documents, (document) => format(new Date(document.datetime), 'yyyy-MM')),
      (it) => it[0].datetime,
    ),
  );
  return (
    <div>
      <div className="flex items-center justify-between gap-4 pb-6">
        <h1 className="flex-1 shrink-0 whitespace-nowrap text-xl font-semibold tracking-tight sm:grow-0">Documents</h1>
        <div className="inline-flex gap-2 rounded-md shadow-sm  bg-gray-100 px-2 py-1 sm:w-auto" role="group">
          <button
            className={`px-2 py-1 text-sm   rounded-md  ${
              layout === 'Grid' ? 'bg-white text-gray-700 shadow-sm font-semibold' : 'bg-transparent text-gray-700 hover:bg-gray-100'
            }`}
            onClick={() => setLayout('Grid')}
          >
            Grid
          </button>
          <button
            className={`px-2 py-1 text-sm  rounded-md  ${
              layout === 'Table' ? 'bg-white text-gray-700 shadow-sm font-semibold' : 'bg-transparent text-gray-700 hover:bg-gray-100'
            }`}
            onClick={() => setLayout('Table')}
          >
            Table
          </button>
        </div>
      </div>
      <ImageLightBox src={lightboxSrc} onChange={setLightboxSrc} />

      {layout === 'Grid' ? (
        <>
          {groupedDocuments.map((targetMonthDocuments, idx) => (
            <>
              <h3 key={`title=${idx}`} className="text-lg font-medium tracking-tight mt-4 mb-4">
                {format(new Date(targetMonthDocuments[0].datetime), 'MMM yyyy')}
              </h3>
              <div key={`grid=${idx}`} className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4 md:gap-6 mt-2">
                {targetMonthDocuments.map((document, idx) => (
                  <AccountDocumentLine onClick={setLightboxSrc} key={idx} {...document} />
                ))}
              </div>
            </>
          ))}
        </>
      ) : (
        <div className="rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Filename</TableHead>
                <TableHead>Linked Directive</TableHead>
                <TableHead>Created Date</TableHead>
                <TableHead>Operation</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {documents.map((document, idx) => (
                <TableRow>
                  <TableCell onClick={isDocumentAnImage(document.path) ? () => setLightboxSrc(document.path) : undefined}>
                    <div>{document.filename}</div>
                  </TableCell>
                  <TableCell>
                    {document.account && (
                      <Badge variant="outline" onClick={() => navigate(`/accounts/${document.account}`)}>
                        {document.account}
                      </Badge>
                    )}
                    {document.trx_id && (
                      <Badge variant="outline" key={idx}>
                        {document.trx_id}
                      </Badge>
                    )}
                  </TableCell>
                  <TableCell>{format(new Date(document.datetime), 'yyyy-MM-dd HH:mm:ss')}</TableCell>
                  <TableCell></TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
    </div>
  );
}
