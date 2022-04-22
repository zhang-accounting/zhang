import { Route, Routes } from "react-router-dom";
import Accounts from "./pages/Accounts";
import Documents from "./pages/Documents";
import Home from "./pages/Home";
import Journals from "./pages/Journals";
import RawEdit from "./pages/RawEdit";
import Report from "./pages/Report";
import SingleAccount from "./pages/SingleAccount";




function App() {

  return (
    <div>
      <div>
        <Routes>
          <Route path="/" element={<Home />} />
          <Route path="journals" element={<Journals />} />
          <Route path="accounts" element={<Accounts />} />
          <Route path="/accounts/:accountName" element={<SingleAccount />} />
          <Route path="documents" element={<Documents />} />
          <Route path="/edit" element={<RawEdit />} />
          <Route path="/report" element={<Report />} />
        </Routes>
      </div>

    </div>
  );
}

export default App;
