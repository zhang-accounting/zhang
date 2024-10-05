import CommodityBox from '../components/CommodityBox';
import { useDocumentTitle } from '@mantine/hooks';
import { useAtomValue } from 'jotai/index';
import { titleAtom } from '../states/basic';
import { FRONTEND_DEFAULT_GROUP, groupedCommoditiesAtom } from '../states/commodity';

export default function Commodities() {
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`Commodities - ${ledgerTitle}`);

  const groupedCommodities = useAtomValue(groupedCommoditiesAtom);

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
