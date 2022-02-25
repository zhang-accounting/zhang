import { useState, useEffect, useRef } from "react";
import logo from "./logo.svg";
import parcel from "./parcel.png";
import "./App.css";

type Timer = ReturnType<typeof setInterval>;

function App() {
  const [count, setCount] = useState(0);
  const [dynamicModuleLoaded, setDynamicModuleLoaded] = useState(false);

  const timer = useRef<Timer>();

  useEffect(() => {
    timer.current = setInterval(() => {
      setCount((c) => c + 1);
    }, 1000);

    import("./dynamic-module").then((res) => {
      console.log("From Dynamic Module: ", res.dynamic());
      setDynamicModuleLoaded(true);
    });

    return () => {
      if (timer.current) clearInterval(timer.current);
    };
  }, []);

  return (
    <div className="App">
      <header className="App-header">
        <div className="App-logo-container">
          <img src={logo} className="App-logo" alt="logo" />
          <img src={parcel} className="Parcel-logo" alt="logo" />
        </div>
        <p>
          Edit <code>src/App.tsx</code> and save to reload.
          <br />
          <br />
          <code>Count: {count}</code>
        </p>
        {dynamicModuleLoaded && <p>Dynamic Module Loaded!</p>}
        <div>
          <a
            className="React-link"
            href="https://reactjs.org"
            target="_blank"
            rel="noopener noreferrer"
          >
            Learn React
          </a>
          <a
            className="Parcel-link"
            href="https://parceljs.org/"
            target="_blank"
            rel="noopener noreferrer"
          >
            Learn Parcel
          </a>
        </div>
      </header>
    </div>
  );
}

export default App;
