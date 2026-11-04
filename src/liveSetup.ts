import { open } from "@tauri-apps/plugin-dialog";
import { getVersion } from "@tauri-apps/api/app";
import { api, NO_USER_LIBRARY } from "./api";

const LIBRARY_TITLE = "Dossier « User Library » de Live (Réglages > Bibliothèque)";

/** Installe le Remote Script et le device de capture. Renvoie un message à afficher. */
export async function installLiveTools(): Promise<string> {
  let script = await api.installRemoteScript(null);
  let library: string | undefined;
  if (!script) {
    const dir = await open({ directory: true, title: LIBRARY_TITLE });
    if (typeof dir !== "string") return "Installation annulée.";
    library = dir;
    script = await api.installRemoteScript(dir);
  }
  let device: string;
  try {
    device = await api.installDevice(library);
  } catch (e) {
    if (String(e) !== NO_USER_LIBRARY) throw e;
    const dir = await open({ directory: true, title: LIBRARY_TITLE });
    if (typeof dir !== "string") return `Script installé (${script}), device annulé.`;
    device = await api.installDevice(dir);
  }
  return (
    `Script installé dans ${script}, device dans ${device}. Redémarre Live, choisis « AbletonManager » ` +
    "dans Préférences > Link, Tempo & MIDI > Control Surface, et glisse « Ableton Manager Capture » sur le Master."
  );
}

/** Vérifie si une version plus récente est publiée sur GitHub. */
export async function checkUpdate(): Promise<{ text: string; url?: string }> {
  const current = await getVersion();
  const res = await fetch("https://api.github.com/repos/atheasstudio/ableton-manager/releases/latest");
  if (res.status === 404)
    return { text: `Version ${current}. Aucune release publiée : mets à jour avec buildabletonmanager.bat.` };
  if (!res.ok) throw new Error(`GitHub répond ${res.status}`);
  const r = await res.json();
  const latest = String(r.tag_name ?? "").replace(/^v/, "");
  if (!latest || latest === current) return { text: `Tu es à jour (${current}).` };
  const asset = (r.assets ?? []).find((a: { name: string }) => /setup\.exe$|\.msi$/i.test(a.name));
  return {
    text: `Nouvelle version ${latest} disponible (tu as ${current}).`,
    url: asset?.browser_download_url ?? r.html_url,
  };
}
