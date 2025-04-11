import { executeSql } from '@/api/requests.ts';
import { Table, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table.tsx';
import { Textarea } from '@/components/ui/textarea';
import { EXPLORE_LINK } from '@/layout/Sidebar.tsx';
import { useDebouncedState, useDocumentTitle } from '@mantine/hooks';
import { useAtomValue, useSetAtom } from 'jotai/index';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useAsync } from 'react-use';
import { breadcrumbAtom, titleAtom } from '../states/basic';


export default function Explore() {
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const { t } = useTranslation();

  const [inputSql, setInputSql] = useDebouncedState('', 200);
  const [executionError, setExecutionError] = useState<string | null>(null);
  const ledgerTitle = useAtomValue(titleAtom);

  useDocumentTitle(`Explore - ${ledgerTitle}`);
  useEffect(() => {
    setBreadcrumb([EXPLORE_LINK]);
  }, []);

  const { value: result } = useAsync(async () => {
    if (inputSql.trim() === '') {
      return {
        columns: [],
        rows: [],
      };
    }
    setExecutionError(null);
    try {
      const res = await executeSql({ sql: inputSql });
      setExecutionError(null);
      return res.data.data;
    } catch (e) {
      console.log("etype", typeof e, e instanceof executeSql.Error);
      if (e instanceof executeSql.Error) {
        // get discriminated union { status, data } 
        const error = e.getActualType()
        console.log("error",error,  error?.data.message);
        if (error.status === 400) {

          setExecutionError(error.data.message ?? "unknown error");
        } else if (error.status === 500) {
          setExecutionError(error.data.message ?? "unknown error");
        } else {
          setExecutionError("unknown error");
        }
      } else {
        setExecutionError("unknown error");
      }
    }
  }, [inputSql]);

  return (
    <div className="container flex flex-col gap-4 mx-auto px-4 py-8">
      <h1 className="text-2xl font-bold mb-6">{t('explore.title')}</h1>

      <div className="space-y-6">
        <Textarea
          defaultValue={inputSql}
          onChange={(event) => setInputSql(event.currentTarget.value)}
        />
      </div>
      {executionError && <div className="text-red-500 mb-2 border border-red-500 rounded-md p-2">
        <pre className="text-red-500">{executionError}</pre>
      </div>}
      <div className="space-y-6">
        {result && (
          <div className="mt-4">
            <h2 className="text-xl font-semibold mb-2">{t('explore.results')}</h2>
            
            <div className="border rounded-md overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow>
                    {result.columns.map((column, index) => (
                      <TableHead key={index}>{column}</TableHead>
                    ))}
                  </TableRow>
                </TableHeader>
                <tbody>
                  {result.rows.map((row, rowIndex) => (
                    <TableRow key={rowIndex}>
                      {row.columns.map((column, colIndex) => (
                        <TableCell key={colIndex}>
                          {/* @ts-ignore */}
                          {column.value} 
                        </TableCell>
                      ))}
                    </TableRow>
                  ))}
                </tbody>
              </Table>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
