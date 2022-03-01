import { Link, Route, Routes } from "react-router-dom";
import "./App.css";
import Accounts from "./pages/Accounts";
import Home from "./pages/Home";
import Journals from "./pages/Journals";




function App() {

  return (
    <div>
      <div>
        <Routes>
          <Route path="/" element={<Home />} />
          <Route path="journals" element={<Journals />} />
          <Route path="accounts" element={<Accounts />} />
        </Routes>
      </div>

    </div>
  );
}

export default App;
