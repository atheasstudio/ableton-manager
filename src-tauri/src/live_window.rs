//! Set ouvert dans Live, lu dans le titre de sa fenêtre : l'API de Live
//! n'expose pas le chemin du fichier.
//!
//! Titre : « Set [Dossier Project] - Ableton Live 12 Suite », avec « * »
//! après le nom du set quand il a des modifications non sauvegardées.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenSet {
    /// Nom du .als, sans extension.
    pub set: String,
    /// Dossier du projet (entre crochets), s'il y en a un.
    pub project: Option<String>,
    /// Modifications non sauvegardées (« * »).
    pub modified: bool,
}

pub fn parse_title(title: &str) -> Option<OpenSet> {
    let (head, _) = title.rsplit_once(" - Ableton Live")?;
    let head = head.trim();
    let (name, project) = match head.strip_suffix(']').and_then(|h| h.rsplit_once(" [")) {
        Some((name, project)) => (name.trim(), Some(project.trim().to_owned())),
        None => (head, None),
    };
    let modified = name.ends_with('*');
    let set = name.trim_end_matches('*').trim().to_owned();
    (!set.is_empty()).then_some(OpenSet { set, project, modified })
}

/// Set ouvert dans Live, si Live tourne.
pub fn current_set() -> Option<OpenSet> {
    window_titles().iter().find_map(|t| parse_title(t))
}

#[cfg(windows)]
fn window_titles() -> Vec<String> {
    use std::ffi::c_void;

    type Hwnd = *mut c_void;
    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(cb: extern "system" fn(Hwnd, isize) -> i32, param: isize) -> i32;
        fn GetWindowTextW(hwnd: Hwnd, buf: *mut u16, max: i32) -> i32;
        fn IsWindowVisible(hwnd: Hwnd) -> i32;
    }

    extern "system" fn collect(hwnd: Hwnd, param: isize) -> i32 {
        // SAFETY : `param` est le pointeur vers le Vec passé à EnumWindows
        // ci-dessous, valable pendant tout l'appel.
        let titles = unsafe { &mut *(param as *mut Vec<String>) };
        let mut buf = [0u16; 512];
        unsafe {
            if IsWindowVisible(hwnd) != 0 {
                let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
                if n > 0 {
                    titles.push(String::from_utf16_lossy(&buf[..n as usize]));
                }
            }
        }
        1
    }

    let mut titles: Vec<String> = vec![];
    unsafe { EnumWindows(collect, &mut titles as *mut Vec<String> as isize) };
    titles
}

#[cfg(not(windows))]
fn window_titles() -> Vec<String> {
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_live_titles() {
        assert_eq!(
            parse_title("woodkid perc* [woodkid perc Project] - Ableton Live 12 Suite"),
            Some(OpenSet {
                set: "woodkid perc".into(),
                project: Some("woodkid perc Project".into()),
                modified: true,
            })
        );
        let loose = parse_title("Sans titre - Ableton Live 11 Standard").unwrap();
        assert_eq!((loose.set.as_str(), loose.project, loose.modified), ("Sans titre", None, false));
        assert_eq!(parse_title("Explorateur de fichiers"), None);
    }
}
