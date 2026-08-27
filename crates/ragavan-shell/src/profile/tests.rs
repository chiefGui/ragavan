use super::*;
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

const INTEGRATION: &[&str] = &["managed integration"];
static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[test]
fn installation_is_idempotent_and_uninstallation_restores_user_content() {
    let directory = TestDirectory::new();
    let profile = directory.path().join("profile");
    let original = b"user-owned content";
    fs::write(&profile, original).expect("profile should be written");

    assert!(install(&profile, INTEGRATION).expect("install should succeed"));
    let installed = fs::read_to_string(&profile).expect("profile should remain UTF-8");
    assert!(installed.starts_with("user-owned content"));
    assert_eq!(installed.matches(START_MARKER).count(), 1);
    assert_eq!(installed.matches(END_MARKER).count(), 1);
    assert!(installed.contains(INTEGRATION[0]));

    let installed_bytes = fs::read(&profile).expect("profile should be readable");
    assert!(!install(&profile, INTEGRATION).expect("reinstall should succeed"));
    assert_eq!(
        fs::read(&profile).expect("profile should be readable"),
        installed_bytes
    );

    assert!(uninstall(&profile).expect("uninstall should succeed"));
    assert_eq!(
        fs::read(&profile).expect("profile should be readable"),
        original
    );
    assert!(!uninstall(&profile).expect("repeated uninstall should succeed"));
}

#[test]
fn installation_updates_only_the_owned_block_and_preserves_newlines() {
    let directory = TestDirectory::new();
    let profile = directory.path().join("profile");
    let original = concat!(
        "before\r\n",
        "\r\n",
        "# >>> ragavan >>>\r\n",
        "stale integration\r\n",
        "# <<< ragavan <<<\r\n",
        "after\r\n",
    );
    fs::write(&profile, original).expect("profile should be written");

    assert!(install(&profile, INTEGRATION).expect("install should succeed"));
    let installed = fs::read_to_string(&profile).expect("profile should be readable");
    assert!(installed.starts_with("before\r\n\r\n"));
    assert!(installed.ends_with("after\r\n"));
    assert!(!installed.contains("stale integration"));
    assert!(!installed.replace("\r\n", "").contains('\n'));

    uninstall(&profile).expect("uninstall should succeed");
    assert_eq!(
        fs::read_to_string(&profile).expect("profile should be readable"),
        "before\r\nafter\r\n"
    );
}

#[test]
fn malformed_markers_are_rejected_without_modifying_the_profile() {
    let directory = TestDirectory::new();
    let profile = directory.path().join("profile");
    let original = b"before\n# >>> ragavan >>>\nmissing end marker\n";
    fs::write(&profile, original).expect("profile should be written");

    let error = install(&profile, INTEGRATION).expect_err("malformed integration should fail");
    assert!(error.to_string().contains("markers are incomplete"));
    assert_eq!(
        fs::read(&profile).expect("profile should be readable"),
        original
    );
    assert_eq!(
        fs::read_dir(directory.path())
            .expect("test directory should be readable")
            .count(),
        1
    );
}

#[test]
fn utf16_encoding_is_preserved() {
    let directory = TestDirectory::new();
    let profile = directory.path().join("profile");
    let original = utf16_little_endian("message = 'olá'\r\n");
    fs::write(&profile, &original).expect("profile should be written");

    install(&profile, INTEGRATION).expect("install should succeed");
    let installed = fs::read(&profile).expect("profile should be readable");
    assert!(installed.starts_with(&[0xff, 0xfe]));
    assert!(decode_utf16_little_endian(&installed).contains(INTEGRATION[0]));

    uninstall(&profile).expect("uninstall should succeed");
    assert_eq!(
        fs::read(&profile).expect("profile should be readable"),
        original
    );
}

#[test]
fn unsupported_encoding_is_rejected_without_modifying_the_profile() {
    let directory = TestDirectory::new();
    let profile = directory.path().join("profile");
    let original = [0x80, 0x81];
    fs::write(&profile, original).expect("profile should be written");

    let error = install(&profile, INTEGRATION).expect_err("unsupported encoding should fail");
    assert!(error.to_string().contains("text encoding is unsupported"));
    assert_eq!(
        fs::read(&profile).expect("profile should be readable"),
        original
    );
}

#[test]
fn installation_creates_a_missing_profile_and_parent_directory() {
    let directory = TestDirectory::new();
    let profile = directory.path().join("missing").join("profile");

    assert!(install(&profile, INTEGRATION).expect("install should succeed"));
    let installed = fs::read_to_string(&profile).expect("profile should be readable");
    assert!(installed.starts_with(START_MARKER));
    assert!(installed.ends_with(if cfg!(windows) { "\r\n" } else { "\n" }));
}

#[cfg(unix)]
#[test]
fn installation_preserves_a_linked_profile() {
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new();
    let target = directory.path().join("dotfiles-profile");
    let profile = directory.path().join("profile");
    let original = b"owned by dotfiles\n";
    fs::write(&target, original).expect("profile target should be written");
    symlink(&target, &profile).expect("profile link should be created");

    install(&profile, INTEGRATION).expect("install should succeed");
    assert!(
        fs::symlink_metadata(&profile)
            .expect("profile link should exist")
            .file_type()
            .is_symlink()
    );
    assert!(
        fs::read_to_string(&target)
            .expect("profile target should be readable")
            .contains(INTEGRATION[0])
    );

    uninstall(&profile).expect("uninstall should succeed");
    assert_eq!(
        fs::read(&target).expect("profile target should be readable"),
        original
    );
}

fn utf16_little_endian(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xfe];
    for code_unit in text.encode_utf16() {
        bytes.extend_from_slice(&code_unit.to_le_bytes());
    }
    bytes
}

fn decode_utf16_little_endian(bytes: &[u8]) -> String {
    assert!(bytes.starts_with(&[0xff, 0xfe]));
    let code_units: Vec<_> = bytes[2..]
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    String::from_utf16(&code_units).expect("profile should remain valid UTF-16")
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        for _ in 0..100 {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "ragavan-profile-test-{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("could not create test directory {path:?}: {error}"),
            }
        }

        panic!("could not allocate a unique test directory");
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!("could not remove test directory {:?}: {error}", self.0);
        }
    }
}
