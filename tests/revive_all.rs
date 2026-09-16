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
