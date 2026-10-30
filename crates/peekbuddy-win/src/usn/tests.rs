use std::os::windows::ffi::OsStrExt as _;
use std::time::{Duration, Instant};

use peekbuddy_core::event::map::usn_reason;
use peekbuddy_core::usn::checkpoint::{ERROR_JOURNAL_NOT_ACTIVE, JournalData};
use peekbuddy_core::usn::record::records;

use super::*;
use crate::io::path::to_long_path;
use crate::io::volume::volume_identity;

/// `ERROR_INVALID_FUNCTION`: a file system without a journal.
const ERROR_INVALID_FUNCTION: i32 = 1;

fn wide(path: &std::path::Path) -> Vec<u16> {
    path.as_os_str().encode_wide().collect()
}

struct Dir(std::path::PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The journal of the volume that holds the temp directory, read only. `None`
/// when that volume has no journal (the test then has nothing to read). The
/// journal itself is never created or deleted here.
fn journal_of(dir: &Dir) -> io::Result<Option<(Journal, JournalData)>> {
    let long = to_long_path(&wide(&dir.0)).unwrap();
    let handle = DirHandle::open(&long, OpenOptions::default()).unwrap();
    let identity = volume_identity(&handle, &long).unwrap();
    let mut journal = Journal::open(&identity.guid_path, DEFAULT_BUFFER).unwrap();
    match journal.query() {
        Ok(data) => Ok(Some((journal, data))),
        Err(error)
            if error.raw_os_error() == Some(ERROR_INVALID_FUNCTION)
                || error.raw_os_error()
                    == Some(i32::try_from(ERROR_JOURNAL_NOT_ACTIVE).unwrap()) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

#[test]
fn the_request_struct_is_serialised_field_by_field() {
    let request = READ_USN_JOURNAL_DATA_V1 {
        StartUsn: 1,
        ReasonMask: 2,
        ReturnOnlyOnClose: 3,
        Timeout: 4,
        BytesToWaitFor: 5,
        UsnJournalID: 6,
        MinMajorVersion: 7,
        MaxMajorVersion: 8,
    };
    let bytes = request_bytes(&request);
    assert_eq!(bytes.len(), size_of::<READ_USN_JOURNAL_DATA_V1>());
    assert_eq!(bytes.len(), 48);
    // The in-memory layout of the struct is the serialised one.
    // SAFETY: `request` is a plain repr(C) struct of integers; reading its
    // bytes for the comparison is sound, the padding bytes are not compared.
    let memory = unsafe {
        std::slice::from_raw_parts(
            std::ptr::from_ref(&request).cast::<u8>(),
            size_of::<READ_USN_JOURNAL_DATA_V1>(),
        )
    };
    assert_eq!(bytes.get(..44), memory.get(..44));
}

#[test]
fn a_created_file_appears_in_the_journal_with_its_reference_number() {
    let dir = Dir(std::env::temp_dir().join(format!("pb-usn-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let Some((mut journal, data)) = journal_of(&dir).unwrap() else {
        return;
    };
    assert!(data.next_usn >= data.first_usn);

    let file = dir.0.join("seen.txt");
    std::fs::write(&file, b"x").unwrap();
    let frn = crate::fs::file_id(&file).unwrap();
    let parent = crate::fs::file_id(&dir.0).unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut next = data.next_usn;
    loop {
        let buf = journal.read(data.journal_id, next).unwrap().to_vec();
        let mut iter = records(&buf).unwrap();
        next = iter.next_usn();
        let found = iter
            .by_ref()
            .flatten()
            .find(|record| record.frn == frn && record.reason & usn_reason::FILE_CREATE != 0);
        if let Some(record) = found {
            assert_eq!(record.parent, parent);
            assert!(!record.is_dir());
            // The unprivileged read carries no names (spike 0001).
            assert!(record.name_withheld() || record.name_units() == wide_name("seen.txt"));
            return;
        }
        assert!(Instant::now() < deadline, "no create record for the file");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wide_name(name: &str) -> Vec<u16> {
    name.encode_utf16().collect()
}

#[test]
fn a_start_usn_in_the_future_is_an_error_not_a_hang() {
    let dir = Dir(std::env::temp_dir().join(format!("pb-usn-future-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let Some((mut journal, data)) = journal_of(&dir).unwrap() else {
        return;
    };
    // Past the end the call returns an empty buffer (the next USN only) or an
    // error; it must not block, because BytesToWaitFor is 0.
    let started = Instant::now();
    let _ = journal.read(data.journal_id, data.next_usn.saturating_add(1 << 40));
    assert!(started.elapsed() < Duration::from_secs(5));
}
