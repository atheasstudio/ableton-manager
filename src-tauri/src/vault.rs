//! Coffre : fichiers rangés par empreinte (BLAKE3), stockés une seule fois.
//!
//! `<coffre>/objects/ab/cdef…` contient le fichier d'empreinte `abcdef…`.
//! Un même sample utilisé par 50 versions ou 300 projets n'occupe donc
//! qu'une place.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub struct Vault {
    root: PathBuf,
}

/// Empreinte d'un fichier lu en entier.
pub fn hash_file(path: &Path) -> io::Result<(String, u64)> {
    let mut hasher = blake3::Hasher::new();
    let size = io::copy(&mut File::open(path)?, &mut hasher)?;
    Ok((hasher.finalize().to_hex().to_string(), size))
}

pub fn hash_bytes(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

impl Vault {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn objects(&self) -> PathBuf {
        self.root.join("objects")
    }

    pub fn object_path(&self, hash: &str) -> PathBuf {
        self.objects().join(&hash[..2]).join(&hash[2..])
    }

    pub fn contains(&self, hash: &str) -> bool {
        self.object_path(hash).is_file()
    }

    /// Range `data` dans le coffre s'il n'y est pas déjà.
    pub fn store_bytes(&self, data: &[u8]) -> io::Result<String> {
        let hash = hash_bytes(data);
        if !self.contains(&hash) {
            let tmp = self.temp_path()?;
            fs::write(&tmp, data)?;
            self.commit(&tmp, &hash)?;
        }
        Ok(hash)
    }

    /// Copie `path` dans le coffre en calculant son empreinte au passage.
    /// L'empreinte renvoyée est celle du contenu réellement copié.
    pub fn store_file(&self, path: &Path) -> io::Result<(String, u64)> {
        let tmp = self.temp_path()?;
        let mut src = File::open(path)?;
        let mut dst = File::create(&tmp)?;
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0; 1 << 20];
        let mut size = 0;
        loop {
            let n = src.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            dst.write_all(&buf[..n])?;
            size += n as u64;
        }
        dst.sync_all()?;
        drop(dst);
        let hash = hasher.finalize().to_hex().to_string();
        if self.contains(&hash) {
            fs::remove_file(&tmp)?;
        } else {
            self.commit(&tmp, &hash)?;
        }
        Ok((hash, size))
    }

    pub fn read(&self, hash: &str) -> io::Result<Vec<u8>> {
        fs::read(self.object_path(hash))
    }

    /// Recrée le fichier `hash` à l'emplacement `dest` (dossiers compris).
    pub fn restore_to(&self, hash: &str, dest: &Path) -> io::Result<()> {
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir)?;
        }
        // On écrit à côté puis on renomme : un fichier à moitié copié
        // ne remplace jamais l'original.
        let tmp = dest.with_extension("am-tmp");
        fs::copy(self.object_path(hash), &tmp)?;
        fs::rename(&tmp, dest)
    }

    /// Supprime les fichiers qui ne sont plus utilisés. Renvoie (fichiers, octets) libérés.
    pub fn collect_garbage(&self, used: &HashSet<String>) -> io::Result<(usize, u64)> {
        let (mut count, mut bytes) = (0, 0);
        for (hash, path, size) in self.list()? {
            if !used.contains(&hash) {
                fs::remove_file(path)?;
                count += 1;
                bytes += size;
            }
        }
        Ok((count, bytes))
    }

    /// (nombre de fichiers, taille totale)
    pub fn usage(&self) -> io::Result<(usize, u64)> {
        let list = self.list()?;
        Ok((list.len(), list.iter().map(|(_, _, s)| s).sum()))
    }

    fn list(&self) -> io::Result<Vec<(String, PathBuf, u64)>> {
        let mut out = vec![];
        let Ok(dirs) = fs::read_dir(self.objects()) else { return Ok(out) };
        for dir in dirs.flatten() {
            let prefix = dir.file_name().to_string_lossy().into_owned();
            if prefix.len() != 2 {
                continue;
            }
            for f in fs::read_dir(dir.path())?.flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                let size = f.metadata().map_or(0, |m| m.len());
                out.push((format!("{prefix}{name}"), f.path(), size));
            }
        }
        Ok(out)
    }

    /// Déplace tout le contenu du coffre vers `dest` (ex. un disque dédié).
    pub fn move_to(&self, dest: &Path) -> io::Result<Vault> {
        let target = Vault::new(dest);
        for (hash, path, _) in self.list()? {
            let to = target.object_path(&hash);
            if to.is_file() {
                fs::remove_file(&path)?;
                continue;
            }
            fs::create_dir_all(to.parent().unwrap())?;
            // Un renommage échoue d'un disque à l'autre : on copie alors.
            if fs::rename(&path, &to).is_err() {
                let tmp = target.temp_path()?;
                fs::copy(&path, &tmp)?;
                target.commit(&tmp, &hash)?;
                fs::remove_file(&path)?;
            }
        }
        Ok(target)
    }

    fn temp_path(&self) -> io::Result<PathBuf> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = self.root.join("tmp");
        fs::create_dir_all(&dir)?;
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        Ok(dir.join(format!("{}-{n}", std::process::id())))
    }

    fn commit(&self, tmp: &Path, hash: &str) -> io::Result<()> {
        let dest = self.object_path(hash);
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::rename(tmp, dest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_once_and_collects_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::new(dir.path().join("coffre"));
        let src = dir.path().join("clap.wav");
        fs::write(&src, b"RIFF clap").unwrap();

        let (h1, size) = vault.store_file(&src).unwrap();
        let (h2, _) = vault.store_file(&src).unwrap();
        assert_eq!(h1, h2);
        assert_eq!(size, 9);
        assert_eq!(h1, hash_file(&src).unwrap().0);
        assert_eq!(vault.store_bytes(b"RIFF clap").unwrap(), h1);
        assert_eq!(vault.usage().unwrap(), (1, 9));

        let out = dir.path().join("ailleurs/Samples/clap.wav");
        vault.restore_to(&h1, &out).unwrap();
        assert_eq!(fs::read(&out).unwrap(), b"RIFF clap");

        let other = vault.store_bytes(b"autre").unwrap();
        let moved = vault.move_to(&dir.path().join("F")).unwrap();
        assert!(moved.contains(&h1) && moved.contains(&other));
        assert_eq!(vault.usage().unwrap().0, 0);

        assert_eq!(moved.collect_garbage(&HashSet::from([h1.clone()])).unwrap(), (1, 5));
        assert!(moved.contains(&h1) && !moved.contains(&other));
    }
}
