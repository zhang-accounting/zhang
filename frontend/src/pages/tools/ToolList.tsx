import { TOOLS_LINK } from '@/layout/Sidebar';
import { useSetAtom } from 'jotai';
import { SquareStack } from 'lucide-react';
import { Link } from 'react-router-dom';
import { breadcrumbAtom } from '@/states/basic';
import { useEffect } from 'react';

const toolItems = [
  {
    title: 'Batch Accounts Balance',
    icon: SquareStack,
    color: 'cyan',
    uri: '/tools/batch-balance',
  },
];

export default function ToolList() {
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  useEffect(() => {
    setBreadcrumb([TOOLS_LINK]);
  }, []);

  const items = toolItems.map((item) => (
    <Link
      to={item.uri}
      key={item.title}
      className="col-span-3 bg-gray-100 dark:bg-dark-700 w-full  h-full p-4 rounded-md transition-all duration-150 ease-in-out relative hover:shadow-md  after:content-[''] after:block after:pb-[100%]"
    >
      <div className="absolute inset-0 w-full h-full flex flex-col items-center justify-center text-center">
        <item.icon className="w-10 h-10" />
        <span className=" mt-4">{item.title}</span>
      </div>
    </Link>
  ));

  return (
    <div>
      <h1 className="flex-1 shrink-0 whitespace-nowrap text-xl font-semibold tracking-tight sm:grow-0">Tools</h1>
      <div className="grid grid-cols-12 gap-4 mt-4">{items}</div>
    </div>
  );
}
