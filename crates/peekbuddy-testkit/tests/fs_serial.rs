//! Sample for the `fs-serial` nextest group: both tests share one fixed root
//! and fail if they ever overlap. `.config/nextest.toml` puts every test
//! binary named `fs_serial*` (and later every watcher, VHD and USN test) into
//! the group.

use std::time::Duration;

use peekbuddy_testkit::{TestDir, base_dir};

fn exclusive_section(who: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(base_dir())?;
    let lock = base_dir().join("fs-serial-sample.lock");
    let created = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock);
    assert!(
        created.is_ok(),
        "{who}: another fs-serial test holds {} ({created:?})",
        lock.display()
    );
    let dir = TestDir::fixed("fs-serial-sample")?;
    std::fs::write(dir.path().join(who), who)?;
    std::thread::sleep(Duration::from_millis(300));
    dir.close()?;
    std::fs::remove_file(&lock)
}

#[test]
fn first_user_of_the_shared_root() {
    exclusive_section("first").unwrap();
}

#[test]
fn second_user_of_the_shared_root() {
    exclusive_section("second").unwrap();
}
