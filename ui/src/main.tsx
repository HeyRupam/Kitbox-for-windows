import React from "react";
import ReactDOM from "react-dom/client";

import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";
import "@fontsource/inter/600.css";
import "@fontsource/space-grotesk/600.css";
import "@fontsource/space-grotesk/700.css";
import "./styles/kitbox.css";

import { App } from "./App";
import { ErrorBoundary } from "./components/ErrorBoundary";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {/* The outer net: if the shell itself fails, this is all that is left. */}
    <ErrorBoundary what="Kitbox" inset>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);
