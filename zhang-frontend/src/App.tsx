import { ApolloClient, InMemoryCache } from "@apollo/client";
import { Link, Route, Routes } from "react-router-dom";
import "./App.css";
import Accounts from "./pages/Accounts";
import Home from "./pages/Home";
import Journals from "./pages/Journals";




function App() {

  return (
    <div>
      <nav
        style={{
          borderBottom: "solid 1px",
          paddingBottom: "1rem",
        }}
      >
        <Link to="/">index</Link> |{" "}
        <Link to="/journals">Journals</Link> |{" "}
        <Link to="/accounts">Accounts</Link>
      </nav>
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
