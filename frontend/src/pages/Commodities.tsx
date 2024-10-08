import CommodityBox from '../components/CommodityBox';
import { useDocumentTitle } from '@mantine/hooks';
import { useAtomValue, useSetAtom } from 'jotai/index';
import { breadcrumbAtom, titleAtom } from '../states/basic';
import { FRONTEND_DEFAULT_GROUP, groupedCommoditiesAtom } from '../states/commodity';
import { COMMODITIES_LINK } from '@/layout/Sidebar';
import { useEffect } from 'react';

export default function Commodities() {
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`Commodities - ${ledgerTitle}`);

  const groupedCommodities = useAtomValue(groupedCommoditiesAtom);
  useEffect(() => {
    setBreadcrumb([COMMODITIES_LINK]);
  }, []);
  return (
    <div>
      {FRONTEND_DEFAULT_GROUP in groupedCommodities && (
        <div className="mt-4">
          <div className="text-sm text-gray-500">Default</div>
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4 md:gap-6 mt-2">
            {groupedCommodities[FRONTEND_DEFAULT_GROUP].map((commodity) => (
              <CommodityBox key={commodity.name} {...commodity} operating_currency={false}></CommodityBox>
            ))}
          </div>
        </div>
      )}
      {Object.keys(groupedCommodities)
        .filter((it) => it !== FRONTEND_DEFAULT_GROUP)
        .sort()
        .map((groupName) => (
          <div className="mt-4">
            <div className="text-sm text-gray-500">{groupName}</div>
            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4 md:gap-6 mt-2">
              {groupedCommodities[groupName].map((commodity) => (
                <CommodityBox {...commodity} operating_currency={false}></CommodityBox>
              ))}
            </div>
          </div>
        ))}
    </div>
  );
}
