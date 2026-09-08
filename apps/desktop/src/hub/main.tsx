/**
 * Hub entry point.
 *
 * React lives here and nowhere near the overlay. The overlay's paint time *is*
 * stage 1 of the latency budget (§4) and a framework mount would sit directly
 * on it; this window opens once, from the tray, and can afford one.
 */
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import "../styles/hub.css";

const root = document.getElementById("root");
if (!root) throw new Error("index.html is missing #root");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
