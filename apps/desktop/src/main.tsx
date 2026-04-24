import React from "react";
import ReactDOM from "react-dom/client";
import { installDevShimIfNeeded } from "./devShim";
import App from "./App";
import "./styles.css";

// Must run before `App` (and therefore before any `invoke()` call
// via Zustand hooks) so the mock is already on `window` by mount.
installDevShimIfNeeded();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
