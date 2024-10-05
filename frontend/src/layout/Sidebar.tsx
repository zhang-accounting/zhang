import { Link, useLocation } from "react-router-dom";
import {
  Bitcoin,
  ChartArea,
  CircleGauge,
  Cog,
  CreditCard,
  FileStack,
  Notebook,
  Package2,
  PencilRuler,
  PocketKnife,
  Receipt,
  RotateCw,
  WalletMinimal,
} from "lucide-react"
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { useTranslation } from "react-i18next";
import { useAtomValue } from "jotai";
import { errorCountAtom } from "@/states/errors";
import { titleAtom, updatableVersionAtom } from "@/states/basic";
import { axiosInstance } from "@/global";
import { notifications } from "@mantine/notifications";



// CircleGauge
const links = [
  { icon: Notebook, label: 'NAV_JOURNALS', uri: '/journals' },
  { icon: WalletMinimal, label: 'NAV_ACCOUNTS', uri: '/accounts' },
  { icon: Receipt, label: 'NAV_COMMODITIES', uri: '/commodities' },
  { icon: Bitcoin, label: 'NAV_BUDGETS', uri: '/budgets' },
  { icon: FileStack, label: 'NAV_DOCUMENTS', uri: '/documents' },
  { icon: ChartArea, label: 'NAV_REPORT', uri: '/report' },
  { icon: CreditCard, label: 'NAV_LIABILITY', uri: '/liability' },
  { icon: PencilRuler, label: 'NAV_RAW_EDITING', uri: '/edit' },
  { icon: PocketKnife, label: 'NAV_TOOLS', uri: '/tools' },
  { icon: Cog, label: 'NAV_SETTING', uri: '/settings' },
];


export default function Sidebar() {

  const { t } = useTranslation();
  const errorsCount = useAtomValue(errorCountAtom);
  const location = useLocation();
  const updatableVersion = useAtomValue(updatableVersionAtom);
  const ledgerTitle = useAtomValue(titleAtom);
  
  
  const refreshLedger = async () => {
    await axiosInstance.post('/api/reload');
  };
  const sendReloadEvent = () => {
    notifications.show({
      id: 'leger-reload',
      title: '[Ledger Reload] reload event is sent',
      message: 'please wait for ledger reload',
      loading: true,
      autoClose: false,
    });
    refreshLedger();
  };
  return (
      <aside className="hidden border-r bg-muted/40 md:block">
        <div className="flex h-full max-h-screen flex-col gap-2">
          <div className="flex h-14 items-center border-b px-4 lg:h-[60px] lg:px-6">
            <div className="flex items-center gap-2 font-semibold">
              <Package2 className="h-6 w-6" />
              <span className="">{ledgerTitle}</span>
            </div>
            <Button variant="ghost" size="icon" className="ml-auto h-8 w-8" onClick={sendReloadEvent}>
              <RotateCw className="h-4 w-4" />
              <span className="sr-only">Refresh Ledger</span>
            </Button>
          </div>
          <div className="flex-1 mt-3">
            <nav className="grid items-start px-2 text-sm font-medium lg:px-4">

            <Link
                to="/"
                className={`flex items-center gap-3 rounded-lg px-3 py-3 text-muted-foreground transition-all hover:text-primary ${location.pathname === '/' ? ' bg-muted text-primary' : ''}`}
              >
                <CircleGauge className="h-4 w-4" />
                {t('NAV_HOME')}
                {errorsCount > 0 && (
                  <Badge className="ml-auto flex h-6 w-6 shrink-0 items-center justify-center rounded-full">
                    {errorsCount}
                  </Badge>
                )}
              </Link>
              {links.map((link) => (
                <Link
                  key={link.label}
                  to={link.uri}
                  className={`flex items-center gap-3 rounded-lg px-3 py-3 text-muted-foreground transition-all hover:text-primary ${location.pathname === link.uri ? ' bg-muted text-primary' : ''}`}
                >
                  <link.icon className="h-4 w-4" />
                  {t(link.label)}
                </Link>
              ))}

            </nav>
          </div>

          {updatableVersion && (
            <div className="mt-auto p-4">
            <Card x-chunk="dashboard-02-chunk-0">
              <CardHeader className="p-2 pt-0 md:p-4">
                <CardTitle>🎉 Version {updatableVersion} is available!</CardTitle>
              </CardHeader>
              <CardContent className="p-2 pt-0 md:p-4 md:pt-0">
                <Link to="https://zhang-accounting.kilerd.me/installation/4-upgrade/" target="_blank">
                  <Button size="sm" className="w-full">
                    Guide to upgrade
                  </Button>
                </Link>
              </CardContent>
            </Card>
          </div>
        )}          
        </div>
      </aside>
     
  )
  ;
}
