use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::save;

const CHARACTER_SUFFIXES: [&str; 3] = [
    "_Header.sav",
    "_Data_WorldFluff.sav",
    "_Data_PlayerLocal.sav",
];

#[derive(Clone, Copy)]
enum BackupKind {
    Revival,
    Restore,
}

impl BackupKind {
    fn suffix(self) -> &'static str {
        match self {
            Self::Revival => "",
            Self::Restore => "-restore",
        }
    }
}

pub struct Character {
    pub name: String,

    path: PathBuf,

    pub deaths: save::DeathStats,

    pub is_dead: bool,

    pub details: save::HeaderDetails,
}

impl Character {
    fn file_stem(&self) -> io::Result<&str> {
        self.path
            .file_name()
            .and_then(OsStr::to_str)
            .and_then(|name| name.strip_suffix("_Header.sav"))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid character path"))
    }
}

fn character_filenames(stem: &str) -> [String; 3] {
    CHARACTER_SUFFIXES.map(|suffix| format!("{stem}{suffix}"))
}

pub fn inspect(
    directory: &Path,
    character: &Character,
) -> io::Result<Vec<(String, Vec<save::PropertyInfo>)>> {
    let stem = character.file_stem()?;
    let mut files = Vec::new();
    for (index, filename) in character_filenames(stem).into_iter().enumerate() {
        match fs::read(directory.join(&filename)) {
            Ok(bytes) => files.push((filename, save::inspect_properties(&bytes)?)),
            Err(error) if error.kind() == io::ErrorKind::NotFound && index != 0 => {}
            Err(error) => return Err(error),
        }
    }
    Ok(files)
}

/// Restores the latest revival backup and preserves the current character files.
pub fn restore(directory: &Path, character: &Character) -> io::Result<(PathBuf, PathBuf)> {
    let stem = character.file_stem()?;
    let filenames = character_filenames(stem);
    let backup_root = backup_root(directory);
    let mut latest = None;
    match fs::read_dir(&backup_root) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                if !entry.file_type()?.is_dir()
                    || entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(BackupKind::Restore.suffix())
                {
                    continue;
                }
                let path = entry.path();
                let header = path.join(&filenames[0]);
                match fs::read(&header) {
                    Ok(bytes) if save::character_name(&bytes)? == character.name => {
                        if latest.as_ref().is_none_or(|current| path > *current) {
                            latest = Some(path);
                        }
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let source = latest.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "No revival backup found for character",
        )
    })?;
    let mut files = Vec::new();
    for (index, filename) in filenames.into_iter().enumerate() {
        match fs::read(source.join(&filename)) {
            Ok(bytes) => files.push((filename, bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound && index != 0 => {}
            Err(error) => return Err(error),
        }
    }
    let snapshot = create_backup_directory(directory, BackupKind::Restore)?;
    let mut staged = Vec::new();
    for (index, (filename, bytes)) in files.iter().enumerate() {
        let temporary = snapshot.join(format!("restore-{index}.tmp"));
        let mut output = File::create_new(&temporary)?;
        output.set_permissions(fs::metadata(source.join(filename))?.permissions())?;
        output.write_all(bytes)?;
        output.sync_all()?;
        staged.push(temporary);
    }
    let mut replaced = Vec::new();
    for ((filename, _), temporary) in files.iter().zip(&staged) {
        let path = directory.join(filename);
        let previous = snapshot.join(filename);
        let existed = match fs::rename(&path, &previous) {
            Ok(()) => true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => {
                rollback_restore(&replaced);
                return Err(error);
            }
        };
        if let Err(error) = fs::rename(temporary, &path) {
            if existed {
                let _ = fs::rename(&previous, &path);
            }
            rollback_restore(&replaced);
            return Err(error);
        }
        replaced.push((path, existed.then_some(previous)));
    }
    Ok((source, snapshot))
}

fn rollback_restore(replaced: &[(PathBuf, Option<PathBuf>)]) {
    for (path, previous) in replaced.iter().rev() {
        let _ = fs::remove_file(path);
        if let Some(previous) = previous {
            let _ = fs::rename(previous, path);
        }
    }
}

pub fn revive(directory: &Path, character: &Character) -> io::Result<PathBuf> {
    let mut bytes = fs::read(&character.path)?;
    let property = save::death_property(&bytes)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Character is no longer dead"))?;
    bytes.drain(property);
    let backup = create_backup_directory(directory, BackupKind::Revival)?;
    let staged = backup.join("revived-header.tmp");
    let mut output = File::create_new(&staged)?;
    output.set_permissions(fs::metadata(&character.path)?.permissions())?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    drop(output);

    let filename = character
        .path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid character path"))?;
    let original = backup.join(filename);
    fs::rename(&character.path, &original)?;
    if let Err(error) = fs::rename(&staged, &character.path) {
        fs::rename(&original, &character.path)?;
        return Err(error);
    }
    Ok(backup)
}

pub fn characters(directory: &Path) -> io::Result<Vec<Character>> {
    let mut characters = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let filename = entry.file_name();
        let Some(stem) = filename
            .to_str()
            .and_then(|name| name.strip_suffix("_Header.sav"))
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        let path = entry.path();
        let bytes = fs::read(&path)?;
        let mut deaths = save::DeathStats {
            count: None,
            play_seconds_at_last_death: None,
        };
        for suffix in &CHARACTER_SUFFIXES[1..] {
            let companion = directory.join(format!("{stem}{suffix}"));
            match fs::read(companion) {
                Ok(bytes) => {
                    let found = save::death_stats(&bytes)?;
                    deaths.count = deaths.count.or(found.count);
                    deaths.play_seconds_at_last_death = deaths
                        .play_seconds_at_last_death
                        .or(found.play_seconds_at_last_death);
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        characters.push(Character {
            name: save::character_name(&bytes)?,
            is_dead: save::death_property(&bytes)?.is_some(),
            details: save::header_details(&bytes)?,
            deaths,
            path,
        });
    }
    characters.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    Ok(characters)
}

fn backup_root(directory: &Path) -> PathBuf {
    let basename = directory
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("SaveGames");
    let parent = directory.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!("{basename}.tq2-hc-revival-backups"))
}

fn create_backup_directory(directory: &Path, kind: BackupKind) -> io::Result<PathBuf> {
    let root = backup_root(directory);
    fs::create_dir_all(&root)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    for attempt in 0..100 {
        let suffix = kind.suffix();
        let path = root.join(format!(
            "{timestamp}-{}-{attempt}{suffix}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Could not create a unique backup directory",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(name: &str, dead: bool) -> Vec<u8> {
        let mut bytes = b"GVAS".to_vec();
        bytes.extend_from_slice(b"\x10\0\0\0m_CharacterName\0");
        bytes.extend_from_slice(b"\x0c\0\0\0StrProperty\0\0\0\0\0");
        bytes.extend_from_slice(&(name.len() as u32 + 5).to_le_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&(name.len() as i32 + 1).to_le_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        if dead {
            bytes.extend_from_slice(b"\x14\0\0\0m_IsPermanentlyDead\0");
            bytes.extend_from_slice(b"\x0d\0\0\0BoolProperty\0\0\0\0\0\0\0\0\0\x01");
            bytes.extend_from_slice(b"\x05\0\0\0None\0");
        }
        bytes
    }

    #[test]
    fn lists_alive_and_dead_characters() {
        let directory = std::env::temp_dir().join(format!(
            "tq2-hc-revival-characters-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("Alive_Header.sav"), header("Alive", false)).unwrap();
        fs::write(directory.join("Dead_Header.sav"), header("Dead", true)).unwrap();

        let found = characters(&directory).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "Alive");
        assert!(!found[0].is_dead);
        assert_eq!(found[1].name, "Dead");
        assert!(found[1].is_dead);

        fs::remove_dir_all(directory).unwrap();
    }
}
