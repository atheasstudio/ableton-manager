//! Fiche d'annuaire (contrat commun, section 1) : à chaque démarrage, l'appli
//! dit aux autres applis d'Atheas où sont sa porte MCP et sa base.
//! Écriture atomique (`.tmp` puis renommage) ; un échec est seulement journalisé.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const NOM_FICHE: &str = "am.json";
const EXE_PORTE: &str = if cfg!(windows) { "ableton-manager-mcp.exe" } else { "ableton-manager-mcp" };

/// Dossier de l'annuaire : `ATHEAS_ANNUAIRE_DIR`, sinon
/// `%LOCALAPPDATA%\Atheas\annuaire` (hors Windows : données XDG).
pub fn dossier() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("ATHEAS_ANNUAIRE_DIR").filter(|d| !d.is_empty()) {
        return Some(d.into());
    }
    let base = if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
    } else {
        match std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
            Some(d) => d.into(),
            None => PathBuf::from(std::env::var_os("HOME")?).join(".local/share"),
        }
    };
    Some(base.join("Atheas").join("annuaire"))
}

/// Chemin de la porte : livrée à côté de l'exe de l'appli (installeur), sinon
/// celle que copie `buildabletonmanager.bat`.
pub fn chemin_porte(dossier_exe: Option<&Path>, localappdata: Option<&Path>) -> PathBuf {
    let voisine = dossier_exe.map(|d| d.join(EXE_PORTE));
    if let Some(v) = voisine.as_ref().filter(|v| v.is_file()) {
        return v.clone();
    }
    match localappdata {
        Some(l) => l.join("AbletonManager").join("mcp").join("ableton-manager-mcp.exe"),
        None => voisine.unwrap_or_else(|| PathBuf::from(EXE_PORTE)),
    }
}

/// Date ISO UTC (`2026-11-10T14:20:00Z`) à partir de secondes depuis 1970.
pub fn iso_utc(secs: i64) -> String {
    let (jours, s) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Algorithme « civil from days » de Howard Hinnant.
    let z = jours + 719_468;
    let ere = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let jour = doy - (153 * mp + 2) / 5 + 1;
    let mois = if mp < 10 { mp + 3 } else { mp - 9 };
    let annee = yoe + ere * 400 + i64::from(mois <= 2);
    format!("{annee:04}-{mois:02}-{jour:02}T{:02}:{:02}:{:02}Z", s / 3600, s % 3600 / 60, s % 60)
}

pub fn fiche(version: &str, exe: Option<&Path>, mcp: &Path, base: &Path, maj: &str) -> Value {
    let mut f = json!({
        "format": 1,
        "appli": "am",
        "nom": "Ableton Manager",
        "version": version,
        "mcp": mcp.to_string_lossy(),
        "base": base.to_string_lossy(),
        "mots": ["morceau", "projet", "ableton", "live", "version", "temps", "plugin", "sample", "tag", "statut"],
        "maj": maj,
    });
    // v1.1 : de quoi relancer l'appli sur une ref (`<exe> --ouvrir am:projet/42`).
    if let Some(exe) = exe {
        f["exe"] = json!(exe.to_string_lossy());
        f["ouvrir_arg"] = json!(crate::ouvrir::ARG);
    }
    f
}

/// Écrit la fiche dans `dossier` : fichier `.tmp` puis renommage.
pub fn ecrire(dossier: &Path, fiche: &Value) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dossier)?;
    let fin = dossier.join(NOM_FICHE);
    let tmp = dossier.join(format!("{NOM_FICHE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(fiche).unwrap_or_default())?;
    std::fs::rename(&tmp, &fin)?;
    Ok(fin)
}

/// Écrit la fiche de l'appli ; ne bloque jamais le démarrage.
pub fn publier(version: &str, base: &Path) {
    let Some(dir) = dossier() else {
        eprintln!("annuaire : dossier introuvable, fiche non écrite");
        return;
    };
    let exe = std::env::current_exe().ok();
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let mcp = chemin_porte(exe.as_deref().and_then(Path::parent), local.as_deref());
    let base = std::path::absolute(base).unwrap_or_else(|_| base.to_path_buf());
    let maintenant = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    if let Err(e) = ecrire(&dir, &fiche(version, exe.as_deref(), &mcp, &base, &iso_utc(maintenant))) {
        eprintln!("annuaire : fiche non écrite dans {} : {e}", dir.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_iso_en_z() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_794_321_600), "2026-11-10T14:40:00Z");
        assert_eq!(iso_utc(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn porte_voisine_sinon_localappdata() {
        let exe = tempfile::tempdir().unwrap();
        let local = Path::new("/local");
        assert_eq!(
            chemin_porte(Some(exe.path()), Some(local)),
            local.join("AbletonManager").join("mcp").join("ableton-manager-mcp.exe")
        );
        std::fs::write(exe.path().join(EXE_PORTE), b"").unwrap();
        assert_eq!(chemin_porte(Some(exe.path()), Some(local)), exe.path().join(EXE_PORTE));
    }

    #[test]
    fn fiche_ecrite_atomiquement() {
        let dir = tempfile::tempdir().unwrap();
        let sous = dir.path().join("annuaire");
        let f = fiche("1.2.3", Some(Path::new("/x/ableton-manager.exe")), Path::new("/x/porte"), Path::new("/x/library.db"), "2026-11-10T14:20:00Z");
        let chemin = ecrire(&sous, &f).unwrap();
        assert_eq!(chemin, sous.join("am.json"));
        assert!(!sous.join("am.json.tmp").exists());
        let lu: Value = serde_json::from_slice(&std::fs::read(&chemin).unwrap()).unwrap();
        assert_eq!(lu["format"], 1);
        assert_eq!(lu["appli"], "am");
        assert_eq!(lu["version"], "1.2.3");
        assert_eq!(lu["base"], "/x/library.db");
        assert_eq!(lu["mots"].as_array().unwrap().len(), 10);
        assert_eq!(lu["exe"], "/x/ableton-manager.exe");
        assert_eq!(lu["ouvrir_arg"], "--ouvrir");
        // Réécrire remplace la fiche.
        ecrire(&sous, &fiche("1.2.4", None, Path::new("/x/porte"), Path::new("/x/library.db"), "z")).unwrap();
        let lu: Value = serde_json::from_slice(&std::fs::read(&chemin).unwrap()).unwrap();
        assert_eq!(lu["version"], "1.2.4");
        assert!(lu.get("exe").is_none());
    }
}
