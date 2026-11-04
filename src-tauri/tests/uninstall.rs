#[path = "../src/uninstall.rs"]
mod uninstall;

#[cfg(windows)]
mod acceptance {
    use super::uninstall;
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "harness-uninstall-中文 空格-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("local")).unwrap();
            fs::create_dir_all(root.join("roaming")).unwrap();
            Self(root)
        }
        fn seed(&self) {
            for root in [
                self.0.join("local/RustHarness"),
                self.0.join("local/dev.rustharness.desktop"),
                self.0.join("roaming/dev.rustharness.desktop"),
            ] {
                fs::create_dir_all(&root).unwrap();
                fs::write(root.join("retain.txt"), "synthetic").unwrap();
            }
        }
        fn purge(&self) -> Result<(), String> {
            uninstall::purge_test_roots(&self.0.join("local"), &self.0.join("roaming"))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn removes_three_fixed_roots_and_preserves_custom_project() {
        let f = Fixture::new();
        f.seed();
        let custom = f.0.join("external custom project");
        fs::create_dir_all(&custom).unwrap();
        fs::write(custom.join("keep.txt"), "external").unwrap();
        f.purge().unwrap();
        assert!(!f.0.join("local/RustHarness").exists());
        assert!(!f.0.join("local/dev.rustharness.desktop").exists());
        assert!(!f.0.join("roaming/dev.rustharness.desktop").exists());
        assert_eq!(
            fs::read_to_string(custom.join("keep.txt")).unwrap(),
            "external"
        );
    }

    #[test]
    fn busy_host_lock_preserves_all_roots() {
        let f = Fixture::new();
        f.seed();
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(f.0.join("local/RustHarness/host.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        assert!(f.purge().is_err());
        assert!(f.0.join("local/RustHarness/retain.txt").exists());
        assert!(
            f.0.join("local/dev.rustharness.desktop/retain.txt")
                .exists()
        );
        assert!(
            f.0.join("roaming/dev.rustharness.desktop/retain.txt")
                .exists()
        );
    }

    fn junction(link: &std::path::Path, target: &std::path::Path) {
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction setup failed: {:?}",
            output
        );
    }

    #[test]
    fn nested_junction_does_not_delete_external_target() {
        let f = Fixture::new();
        f.seed();
        let outside = f.0.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), "external").unwrap();
        junction(&f.0.join("local/RustHarness/project-link"), &outside);
        fs::create_dir_all(f.0.join("local/RustHarness/nested")).unwrap();
        junction(&f.0.join("local/RustHarness/nested/deep-link"), &outside);
        junction(
            &f.0.join("local/dev.rustharness.desktop/project-link"),
            &outside,
        );
        f.purge().unwrap();
        assert_eq!(
            fs::read_to_string(outside.join("keep.txt")).unwrap(),
            "external"
        );
    }

    #[test]
    fn reparse_root_is_refused_before_any_removal() {
        let f = Fixture::new();
        f.seed();
        let outside = f.0.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), "external").unwrap();
        fs::remove_dir_all(f.0.join("roaming/dev.rustharness.desktop")).unwrap();
        junction(&f.0.join("roaming/dev.rustharness.desktop"), &outside);
        assert!(f.purge().is_err());
        assert!(f.0.join("local/RustHarness/retain.txt").exists());
        assert_eq!(
            fs::read_to_string(outside.join("keep.txt")).unwrap(),
            "external"
        );
    }

    #[test]
    fn later_staging_failure_rolls_back_earlier_entries() {
        use std::os::windows::fs::OpenOptionsExt;
        let f = Fixture::new();
        f.seed();
        // Windows forbids moving this later root with a child handle open.
        let _busy = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(f.0.join("roaming/dev.rustharness.desktop/retain.txt"))
            .unwrap();
        assert!(f.purge().is_err());
        assert_eq!(
            fs::read_to_string(f.0.join("local/RustHarness/retain.txt")).unwrap(),
            "synthetic"
        );
        assert_eq!(
            fs::read_to_string(f.0.join("local/dev.rustharness.desktop/retain.txt")).unwrap(),
            "synthetic"
        );
        assert!(
            f.0.join("roaming/dev.rustharness.desktop/retain.txt")
                .exists()
        );
        assert!(!fs::read_dir(f.0.join("local")).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".rust-harness-uninstall-")
        }));
    }
}
