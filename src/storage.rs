use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::save;

pub struct Character {
    pub name: String,

    path: PathBuf,

    pub deaths: save::DeathStats,

    pub is_dead: bool,

    pub details: save::HeaderDetails,
}

pub fn revive(directory: &Path, character: &Character) -> io::Result<PathBuf> {
    let mut bytes = fs::read(&character.path)?;
    let property = save::death_property(&bytes)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Character is no longer dead"))?;
    bytes.drain(property);
    let mut edits = vec![(character.path.clone(), bytes)];
    let filename = character
        .path
        .file_name()
        .and_then(OsStr::to_str)
        .and_then(|name| name.strip_suffix("_Header.sav"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid character path"))?;
    for suffix in ["_Data_WorldFluff.sav", "_Data_PlayerLocal.sav"] {
        let path = directory.join(format!("{filename}{suffix}"));
        match fs::read(&path) {
            Ok(mut bytes) => {
                if save::remove_death_stats(&mut bytes)? {
                    edits.push((path, bytes));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    let cleanup = cleanup_files(directory)?;
    let backup = create_backup_directory(directory)?;
    let mut staged = Vec::with_capacity(edits.len());
    for (index, (path, bytes)) in edits.iter().enumerate() {
        let temporary = backup.join(format!("revived-{index}.tmp"));
        let mut output = File::create_new(&temporary)?;
        output.set_permissions(fs::metadata(path)?.permissions())?;
        output.write_all(bytes)?;
        output.sync_all()?;
        staged.push(temporary);
    }
    let mut replaced = Vec::with_capacity(edits.len());
    for ((path, _), temporary) in edits.iter().zip(&staged) {
        let filename = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid character path"))?;
        let original = backup.join(filename);
        if let Err(error) = fs::rename(path, &original).and_then(|()| {
            fs::rename(temporary, path).inspect_err(|_| {
                let _ = fs::rename(&original, path);
            })
        }) {
            for (path, original) in replaced.into_iter().rev() {
                let _ = fs::remove_file(&path);
                let _ = fs::rename(original, path);
            }
            return Err(error);
        }
        replaced.push((path.clone(), original));
    }
    for file in cleanup {
        let filename = file.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Invalid cleanup file path")
        })?;
        fs::rename(&file, backup.join(filename))?;
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
        for suffix in ["_Data_WorldFluff.sav", "_Data_PlayerLocal.sav"] {
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

fn cleanup_files(directory: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension() == Some(OsStr::new("bak"))
            || entry.file_name() == OsStr::new("Saving.sav")
        {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn create_backup_directory(directory: &Path) -> io::Result<PathBuf> {
    let basename = directory
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("SaveGames");
    let parent = directory.parent().unwrap_or_else(|| Path::new("."));
    let root = parent.join(format!("{basename}.tq2-hc-revival-backups"));
    fs::create_dir_all(&root)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    for attempt in 0..100 {
        let path = root.join(format!("{timestamp}-{}-{attempt}", std::process::id()));
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
