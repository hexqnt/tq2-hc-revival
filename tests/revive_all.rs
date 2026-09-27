use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

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

fn scalar(bytes: &mut Vec<u8>, name: &str, kind: &str, value: &[u8]) {
    for text in [name, kind] {
        bytes.extend_from_slice(&(text.len() as u32 + 1).to_le_bytes());
        bytes.extend_from_slice(text.as_bytes());
        bytes.push(0);
    }
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
    bytes.push(0);
    bytes.extend_from_slice(value);
}

#[test]
fn revive_preserves_companion_saves_and_shared_files() {
    let root = std::env::temp_dir().join(format!(
        "tq2-hc-revival-stats-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let directory = root.join("SaveGames");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("Hero_Header.sav"), header("Hero", true)).unwrap();
    let mut world = b"GVAS".to_vec();
    scalar(
        &mut world,
        "m_DeathCounter",
        "IntProperty",
        &3u32.to_le_bytes(),
    );
    scalar(&mut world, "m_Keep", "IntProperty", &42u32.to_le_bytes());
    let mut local = b"GVAS".to_vec();
    scalar(
        &mut local,
        "m_TotalPlaytimeAtLastDeath",
        "DoubleProperty",
        &123.5f64.to_le_bytes(),
    );
    scalar(&mut local, "m_Keep", "IntProperty", &21u32.to_le_bytes());
    fs::write(directory.join("Hero_Data_WorldFluff.sav"), &world).unwrap();
    fs::write(directory.join("Hero_Data_PlayerLocal.sav"), &local).unwrap();
    fs::write(directory.join("Hero_Data_PlayerLocal.bak"), b"map backup").unwrap();
    fs::write(directory.join("Saving.sav"), b"saving state").unwrap();

    let list = Command::new(env!("CARGO_BIN_EXE_tq2-hc-revival"))
        .args(["--save-dir", directory.to_str().unwrap(), "ls"])
        .output()
        .unwrap();
    assert!(list.status.success());
    assert!(String::from_utf8_lossy(&list.stdout).contains("Hero"));

    let inspect = Command::new(env!("CARGO_BIN_EXE_tq2-hc-revival"))
        .args(["--save-dir", directory.to_str().unwrap(), "inspect", "Hero"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(inspect.status.success());
    let inspected = String::from_utf8_lossy(&inspect.stdout);
    assert!(inspected.contains("m_DeathCounter (IntProperty) = 3"));
    assert!(inspected.contains("m_TotalPlaytimeAtLastDeath (DoubleProperty) = 123.5"));

    let unavailable = Command::new(env!("CARGO_BIN_EXE_tq2-hc-revival"))
        .args(["--save-dir", directory.to_str().unwrap(), "restore", "Hero"])
        .output()
        .unwrap();
    assert!(!unavailable.status.success());
    assert_eq!(
        fs::read(directory.join("Hero_Header.sav")).unwrap(),
        header("Hero", true)
    );

    let output = Command::new(env!("CARGO_BIN_EXE_tq2-hc-revival"))
        .args(["--save-dir", directory.to_str().unwrap(), "revive", "Hero"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let backup_root = root.join("SaveGames.tq2-hc-revival-backups");
    let backup = fs::read_dir(backup_root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        fs::read(backup.join("Hero_Header.sav")).unwrap(),
        header("Hero", true)
    );
    assert!(!backup.join("Hero_Data_WorldFluff.sav").exists());
    assert!(!backup.join("Hero_Data_PlayerLocal.sav").exists());
    assert_eq!(
        fs::read(directory.join("Hero_Data_WorldFluff.sav")).unwrap(),
        world
    );
    assert_eq!(
        fs::read(directory.join("Hero_Data_PlayerLocal.sav")).unwrap(),
        local
    );
    assert_eq!(
        fs::read(directory.join("Hero_Data_PlayerLocal.bak")).unwrap(),
        b"map backup"
    );
    assert_eq!(
        fs::read(directory.join("Saving.sav")).unwrap(),
        b"saving state"
    );
    let list = Command::new(env!("CARGO_BIN_EXE_tq2-hc-revival"))
        .args(["--save-dir", directory.to_str().unwrap(), "ls"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(list.status.success());
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(
        stdout
            .lines()
            .any(|line| line.contains("Hero") && line.contains("3"))
    );
    let restore = Command::new(env!("CARGO_BIN_EXE_tq2-hc-revival"))
        .args(["--save-dir", directory.to_str().unwrap(), "restore", "Hero"])
        .output()
        .unwrap();
    assert!(
        restore.status.success(),
        "{}",
        String::from_utf8_lossy(&restore.stderr)
    );
    assert_eq!(
        fs::read(directory.join("Hero_Header.sav")).unwrap(),
        header("Hero", true)
    );
    assert_eq!(
        fs::read(directory.join("Hero_Data_WorldFluff.sav")).unwrap(),
        world
    );
    assert_eq!(
        fs::read(directory.join("Hero_Data_PlayerLocal.sav")).unwrap(),
        local
    );
    assert_eq!(
        fs::read_dir(root.join("SaveGames.tq2-hc-revival-backups"))
            .unwrap()
            .count(),
        2
    );
    let snapshot = fs::read_dir(root.join("SaveGames.tq2-hc-revival-backups"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("-restore")
        })
        .unwrap();
    assert_ne!(
        fs::read(snapshot.join("Hero_Header.sav")).unwrap(),
        header("Hero", true)
    );
    assert!(!snapshot.join("Hero_Data_WorldFluff.sav").exists());
    assert!(!snapshot.join("Hero_Data_PlayerLocal.sav").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn list_hides_zero_death_statistics_for_already_revived_characters() {
    let root = std::env::temp_dir().join(format!(
        "tq2-hc-revival-old-stats-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    fs::write(root.join("Hero_Header.sav"), header("Hero", false)).unwrap();
    let mut stats = b"GVAS".to_vec();
    scalar(
        &mut stats,
        "m_DeathCounter",
        "IntProperty",
        &0u32.to_le_bytes(),
    );
    scalar(
        &mut stats,
        "m_TotalPlaytimeAtLastDeath",
        "DoubleProperty",
        &0f64.to_le_bytes(),
    );
    fs::write(root.join("Hero_Data_WorldFluff.sav"), stats).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tq2-hc-revival"))
        .args(["--save-dir", root.to_str().unwrap(), "ls"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.lines().any(|line| {
        line.contains("Hero")
            && line
                .split_whitespace()
                .rev()
                .take(2)
                .all(|field| field == "—")
    }));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn revive_all_only_changes_dead_characters() {
    let root = std::env::temp_dir().join(format!(
        "tq2-hc-revival-all-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let directory = root.join("SaveGames");
    fs::create_dir_all(&directory).unwrap();
    let alive = header("Alive", false);
    fs::write(directory.join("Alive_Header.sav"), &alive).unwrap();
    for name in ["all", "First", "Second"] {
        fs::write(
            directory.join(format!("{name}_Header.sav")),
            header(name, true),
        )
        .unwrap();
    }

    let command = |target: &str| {
        Command::new(env!("CARGO_BIN_EXE_tq2-hc-revival"))
            .args(["--save-dir", directory.to_str().unwrap(), "revive", target])
            .output()
            .unwrap()
    };
    let output = command("all");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Revived all."));
    assert!(!stdout.contains("Revived First."));
    assert_eq!(
        fs::read_dir(root.join("SaveGames.tq2-hc-revival-backups"))
            .unwrap()
            .count(),
        1
    );
    assert!(
        fs::read(directory.join("First_Header.sav"))
            .unwrap()
            .windows(b"m_IsPermanentlyDead".len())
            .any(|window| window == b"m_IsPermanentlyDead")
    );

    let output = command("--all");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Revived First."));
    assert!(stdout.contains("Revived Second."));
    assert!(!stdout.contains("Revived Alive."));
    assert_eq!(fs::read(directory.join("Alive_Header.sav")).unwrap(), alive);
    for name in ["all", "First", "Second"] {
        let bytes = fs::read(directory.join(format!("{name}_Header.sav"))).unwrap();
        assert!(
            !bytes
                .windows(b"m_IsPermanentlyDead".len())
                .any(|window| window == b"m_IsPermanentlyDead")
        );
    }
    let backup_root = root.join("SaveGames.tq2-hc-revival-backups");
    assert_eq!(fs::read_dir(&backup_root).unwrap().count(), 3);

    let output = command("--all");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("No dead Hardcore characters found"));
    assert_eq!(fs::read_dir(&backup_root).unwrap().count(), 3);

    fs::remove_dir_all(root).unwrap();
}
#[test]
fn interactive_selection_can_be_quit_without_reviving() {
    let root = std::env::temp_dir().join(format!(
        "tq2-hc-revival-quit-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let directory = root.join("SaveGames");
    fs::create_dir_all(&directory).unwrap();
    let original = header("Hero", true);
    let save = directory.join("Hero_Header.sav");
    fs::write(&save, &original).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_tq2-hc-revival"))
        .args(["--save-dir", directory.to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"q\n").unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("(q to quit)"));
    assert_eq!(fs::read(save).unwrap(), original);
    assert!(!root.join("SaveGames.tq2-hc-revival-backups").exists());
    fs::remove_dir_all(root).unwrap();
}
