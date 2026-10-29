import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import DoctorDialog from "./components/DoctorDialog";
import { listen } from "@tauri-apps/api/event";
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
