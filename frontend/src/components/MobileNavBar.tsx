import React from 'react';
import { Link, useLocation } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { 
  LayoutDashboard, 
  BookOpen, 
  Wallet, 
  PieChart, 
  Settings 
} from 'lucide-react';

const MobileNavBar: React.FC = () => {
  const { t } = useTranslation();
  const location = useLocation();
  
  const isActive = (path: string) => {
    return location.pathname === path || location.pathname.startsWith(`${path}/`);
  };
  
  const navItems = [
    { 
      path: '/', 
      label: t('NAV_DASHBOARD'), 
      icon: LayoutDashboard 
    },
    { 
      path: '/journals', 
      label: t('NAV_JOURNALS'), 
      icon: BookOpen 
    },
    { 
      path: '/accounts', 
      label: t('NAV_ACCOUNTS'), 
      icon: Wallet 
    },
    { 
      path: '/budgets', 
      label: t('NAV_BUDGETS'), 
      icon: PieChart 
    },
    { 
      path: '/settings', 
      label: t('NAV_SETTING'), 
      icon: Settings 
    }
  ];
  
  return (
    <div className="sm:hidden fixed bottom-0 left-0 right-0 bg-background border-t border-border z-50">
      <div className="flex justify-around items-center h-16">
        {navItems.map((item) => (
          <Link 
            key={item.path} 
            to={item.path} 
            className={`flex flex-col items-center justify-center w-full h-full ${
              isActive(item.path) 
                ? 'text-primary' 
                : 'text-muted-foreground'
            }`}
          >
            <item.icon className="h-5 w-5 mb-1" />
            <span className="text-xs">{item.label}</span>
          </Link>
        ))}
      </div>
    </div>
  );
};

export default MobileNavBar; 