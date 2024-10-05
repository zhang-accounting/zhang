import useSWR from 'swr';
import { fetcher } from '../global.ts';
import SingleFileEdit from '../components/SingleFileEdit';
import { TableOfContentsFloating, Tier, ZHANG_VALUE } from '../components/basic/TableOfContentsFloating';
import { useState } from 'react';
import { useDocumentTitle } from '@mantine/hooks';
import { useAtomValue } from 'jotai/index';
import { titleAtom } from '../states/basic';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card.tsx';



function RawEdit() {
  const { data, error } = useSWR<string[]>('/api/files', fetcher);
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(selectedFile ? `${selectedFile} | Raw Editing - ${ledgerTitle}` : `Raw Editing - ${ledgerTitle}`);

  if (error) return <div>failed to load</div>;
  if (!data) return <>loading</>;

  const tree: Tier = {};
  data
    .map((it) => it.replace(/^\/|\/$/g, ''))
    .forEach((entry) => {
      let ref_tree = tree;
      entry.split(/[/\\]/).forEach((path) => {
        if (!ref_tree.hasOwnProperty(path)) {
          ref_tree[path] = {};
        }
        ref_tree = ref_tree[path];
      });
      ref_tree[ZHANG_VALUE] = entry;
    });
  return (
    <Card className="rounded-sm">
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2 ">
        <CardTitle>File: {selectedFile}</CardTitle>
        <TableOfContentsFloating files={tree} onChange={(value) => setSelectedFile(value)} />
      </CardHeader>
      <CardContent>
        {selectedFile && <SingleFileEdit name={selectedFile} path={selectedFile} />}
      </CardContent>
    </Card>
  );
}

export default RawEdit;
