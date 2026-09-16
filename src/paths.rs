use std::{
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

const SAVE_PATH: &str = "TQ2/Saved/SaveGames";
#[cfg(target_os = "linux")]
const APP_ID: &str = "1154030";

enum VdfToken {
    Text(String),

    Open,

    Close,
}

struct VdfTokens<'a> {
    rest: &'a str,
}

impl<'a> VdfTokens<'a> {
    fn new(contents: &'a str) -> Self {
        Self { rest: contents }
    }
}

impl Iterator for VdfTokens<'_> {
    type Item = VdfToken;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.rest = self.rest.trim_start();
            if let Some(comment) = self.rest.strip_prefix("//") {
                self.rest = comment.split_once('\n').map_or("", |(_, rest)| rest);
                continue;
            }
            let first = self.rest.chars().next()?;
            self.rest = &self.rest[first.len_utf8()..];
            match first {
                '{' => return Some(VdfToken::Open),
                '}' => return Some(VdfToken::Close),
                '"' => {
                    let mut value = String::new();
                    let mut chars = self.rest.char_indices();
                    while let Some((index, ch)) = chars.next() {
                        match ch {
                            '"' => {
                                self.rest = &self.rest[index + 1..];
                                return Some(VdfToken::Text(value));
                            }
                            '\\' => {
                                if let Some((_, escaped)) = chars.next() {
                                    value.push(escaped);
                                }
                            }
                            _ => value.push(ch),
                        }
                    }
                    self.rest = "";
                    return None;
                }
                _ => {}
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn home_dir() -> Option<PathBuf> {
    env::var_os("USERPROFILE").map(PathBuf::from)
}

#[cfg(not(target_os = "windows"))]
fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn add_bottles(directories: &mut Vec<PathBuf>, root: &Path) {
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            add_wine_prefix(directories, &entry.path());
        }
    }
}

fn add_directory(directories: &mut Vec<PathBuf>, path: PathBuf) {
    if let Ok(path) = path.canonicalize()
        && path.is_dir()
        && !directories.iter().any(|existing| existing == &path)
    {
        directories.push(path);
    }
}

fn add_steam_root(directories: &mut Vec<PathBuf>, root: &Path) {
    let mut libraries = vec![root.to_path_buf()];
    let config = root.join("steamapps/libraryfolders.vdf");
    if let Ok(contents) = fs::read_to_string(config) {
        libraries.extend(steam_library_paths(&contents));
    }
    for library in libraries {
        let steamapps = library.join("steamapps");
        add_directory(
            directories,
            steamapps.join("common/Titan Quest II").join(SAVE_PATH),
        );
        #[cfg(target_os = "linux")]
        add_wine_prefix(
            directories,
            &steamapps.join("compatdata").join(APP_ID).join("pfx"),
        );
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn add_wine_prefix(directories: &mut Vec<PathBuf>, prefix: &Path) {
    let users = prefix.join("drive_c/users");
    if let Ok(entries) = fs::read_dir(users) {
        for entry in entries.flatten() {
            add_directory(
                directories,
                entry.path().join("AppData/Local").join(SAVE_PATH),
            );
        }
    }
}

pub fn discover_save_dirs() -> Vec<PathBuf> {
    let mut directories = Vec::new();

    #[cfg(target_os = "windows")]
    {
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            add_directory(&mut directories, PathBuf::from(local).join(SAVE_PATH));
        }
        if let Some(home) = home_dir() {
            add_directory(&mut directories, home.join("AppData/Local").join(SAVE_PATH));
        }
        for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(program_files) = env::var_os(variable) {
                add_steam_root(
                    &mut directories,
                    &PathBuf::from(program_files).join("Steam"),
                );
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(prefix) = env::var_os("STEAM_COMPAT_DATA_PATH") {
            add_wine_prefix(&mut directories, &PathBuf::from(prefix).join("pfx"));
        }
        if let Some(data) = env::var_os("XDG_DATA_HOME") {
            add_steam_root(&mut directories, &PathBuf::from(data).join("Steam"));
        }
        if let Some(home) = home_dir() {
            for root in [
                ".local/share/Steam",
                ".steam/steam",
                ".steam/root",
                ".var/app/com.valvesoftware.Steam/.local/share/Steam",
            ] {
                add_steam_root(&mut directories, &home.join(root));
            }
        }
    }

    #[cfg(target_os = "macos")]
    if let Some(home) = home_dir() {
        add_directory(
            &mut directories,
            home.join("Library/Application Support").join(SAVE_PATH),
        );
        add_steam_root(
            &mut directories,
            &home.join("Library/Application Support/Steam"),
        );
        for bottles in [
            "Library/Application Support/CrossOver/Bottles",
            "Library/Containers/com.isaacmarovitz.Whisky/Bottles",
        ] {
            add_bottles(&mut directories, &home.join(bottles));
        }
    }

    let populated: Vec<_> = directories
        .iter()
        .filter(|path| has_character_headers(path))
        .cloned()
        .collect();
    if populated.is_empty() {
        directories
    } else {
        populated
    }
}

fn steam_library_paths(contents: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut pending: Option<String> = None;
    for token in VdfTokens::new(contents) {
        match token {
            VdfToken::Text(value) => {
                if let Some(key) = pending.take() {
                    let legacy = stack.len() == 1 && key.parse::<u32>().is_ok();
                    let modern =
                        stack.len() == 2 && stack[1].parse::<u32>().is_ok() && key == "path";
                    if stack.first().is_some_and(|root| root == "libraryfolders")
                        && (legacy || modern)
                    {
                        paths.push(PathBuf::from(value));
                    }
                } else {
                    pending = Some(value);
                }
            }
            VdfToken::Open => {
                if let Some(key) = pending.take() {
                    stack.push(key);
                }
            }
            VdfToken::Close => {
                stack.pop();
                pending = None;
            }
        }
    }
    paths
}

fn has_character_headers(directory: &Path) -> bool {
    fs::read_dir(directory).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file())
                && entry
                    .path()
                    .file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|name| name.ends_with("_Header.sav"))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_modern_and_legacy_steam_libraries() {
        let contents = r#"
            "libraryfolders"
            {
                "0" { "path" "/home/user/.local/share/Steam" "apps" { "1154030" "1" } }
                "1" { "path" "/mnt/Games\\SteamLibrary" }
                "2" "/mnt/other"
            }
        "#;
        assert_eq!(
            steam_library_paths(contents),
            [
                PathBuf::from("/home/user/.local/share/Steam"),
                PathBuf::from("/mnt/Games\\SteamLibrary"),
                PathBuf::from("/mnt/other"),
            ]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn finds_proton_save_in_secondary_steam_library() {
        let root = env::temp_dir().join(format!(
            "tq2-hc-revival-paths-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let steam = root.join("Steam");
        let library = root.join("Games");
        let saves = library.join(format!(
            "steamapps/compatdata/{APP_ID}/pfx/drive_c/users/steamuser/AppData/Local/{SAVE_PATH}"
        ));
        fs::create_dir_all(steam.join("steamapps")).unwrap();
        fs::create_dir_all(&saves).unwrap();
        fs::write(
            steam.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} }}",
                library.display()
            ),
        )
        .unwrap();

        let mut found = Vec::new();
        add_steam_root(&mut found, &steam);
        assert_eq!(found, [saves]);

        fs::remove_dir_all(root).unwrap();
    }
}
