import { Route, Routes } from 'react-router-dom';
import Home from './pages/Home';
import Journals from './pages/Journals';
import Accounts from './pages/Accounts';
import SingleAccount from './pages/SingleAccount';
import Commodities from './pages/Commodities';
import SingleCommodity from './pages/SingleCommodity';
import Documents from './pages/Documents';
import Budgets from './pages/Budgets';
import SingleBudget from './pages/SingleBudget';
import RawEdit from './pages/RawEdit';
import Report from './pages/Report';
import ToolList from './pages/tools/ToolList';
import WechatExporter from './pages/tools/WechatExporter';
import BatchBalance from './pages/tools/BatchBalance';
import Settings from './pages/Settings';

export function Router() {
  return (
    <Routes>
      <Route path="/" element={<Home />} />
      <Route path="/journals" element={<Journals />} />
      <Route path="/accounts" element={<Accounts />} />
      <Route path="/accounts/:accountName" element={<SingleAccount />} />
      <Route path="/commodities" element={<Commodities />} />
      <Route path="/commodities/:commodityName" element={<SingleCommodity />} />
      <Route path="/documents" element={<Documents />} />
      <Route path="/budgets" element={<Budgets />} />
      <Route path="/budgets/:budgetName" element={<SingleBudget />} />
      <Route path="/edit" element={<RawEdit />} />
      <Route path="/report" element={<Report />} />
      <Route path="/tools" element={<ToolList />} />
      <Route path="/tools/wechat-exporter" element={<WechatExporter />} />
      <Route path="/tools/batch-balance" element={<BatchBalance />} />
      <Route path="/settings" element={<Settings />} />
    </Routes>
  );
}
