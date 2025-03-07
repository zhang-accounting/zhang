import { retrieveFiles } from '@/api/requests.ts';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card.tsx';
import { RAW_EDITING_LINK } from '@/layout/Sidebar';
import { useDocumentTitle } from '@mantine/hooks';
import { useAtomValue, useSetAtom } from 'jotai/index';
import { useEffect, useState } from 'react';
import { useAsync } from 'react-use';
import SingleFileEdit from '../components/SingleFileEdit';
import { TableOfContentsFloating, Tier, ZHANG_VALUE } from '../components/basic/TableOfContentsFloating';
import { breadcrumbAtom, titleAtom } from '../states/basic';

function RawEdit() {
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const {
    loading,
    error,
    value: data,
  } = useAsync(async () => {
    const res = await retrieveFiles({});
    return res.data.data;
  }, []);
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(selectedFile ? `${selectedFile} | Raw Editing - ${ledgerTitle}` : `Raw Editing - ${ledgerTitle}`);
  useEffect(() => {
    setBreadcrumb([RAW_EDITING_LINK]);
  }, []);
  if (error) return <div>failed to load</div>;
  if (loading || !data) return <div>loading...</div>;

  const tree: Tier = {};
  data
    .filter((it) => it !== null)
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
      <CardContent>{selectedFile && <SingleFileEdit name={selectedFile} path={selectedFile} />}</CardContent>
    </Card>
  );
}

export default RawEdit;
