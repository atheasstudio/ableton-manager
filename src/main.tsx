import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import DoctorDialog from "./components/DoctorDialog";
import { listen } from "@tauri-apps/api/event";
import "@fontsource/chakra-petch/500.css";
import "@fontsource/chakra-petch/700.css";
import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "./atheas-charte.css";
import "./styles.css";

// Scan lancé depuis l'icône de la zone de notification : on annonce le résultat.
listen<{ message: string }>("tray-scan-done", (e) =>
  window.dispatchEvent(new CustomEvent("am:toast", { detail: { message: e.payload.message } })),
).catch(() => {});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
    <DoctorDialog />
  </React.StrictMode>,
);
