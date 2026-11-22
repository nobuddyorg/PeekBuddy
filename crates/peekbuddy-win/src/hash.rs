//! The SHA-256 of a file, read with a fixed buffer and never recalled from
//! the cloud (#85, `peekbuddy_core::filehash`).
//!
//! The file is opened for reading with every share mode, so a program that
//! writes or deletes it is never blocked, and with `FILE_FLAG_OPEN_NO_RECALL`;
//! a placeholder is refused before it is opened. The digest comes from the
//! system's SHA-256 (`BCrypt`), fed one buffer at a time.

use std::io::Read as _;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::fs::OpenOptionsExt as _;
use std::path::Path;

use peekbuddy_core::filehash::{CHUNK, Outcome, is_placeholder};
use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_HASH_HANDLE, BCRYPT_SHA256_ALG_HANDLE, BCryptCreateHash, BCryptDestroyHash,
    BCryptFinishHash, BCryptHashData,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_SEQUENTIAL_SCAN, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, GetFileAttributesW, INVALID_FILE_ATTRIBUTES,
};

fn status(code: i32, call: &str) -> std::io::Result<()> {
    if code < 0 {
        Err(std::io::Error::other(format!(
            "{call} failed: NTSTATUS {code:#010x}"
        )))
    } else {
        Ok(())
    }
}

/// A running SHA-256.
struct Sha256(BCRYPT_HASH_HANDLE);

impl Sha256 {
    fn new() -> std::io::Result<Self> {
        let mut handle: BCRYPT_HASH_HANDLE = std::ptr::null_mut();
        // SAFETY: the SHA-256 pseudo-handle needs no provider; BCrypt
        // allocates the hash object itself when none is passed.
        let code = unsafe {
            BCryptCreateHash(
                BCRYPT_SHA256_ALG_HANDLE,
                &raw mut handle,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
                0,
                0,
            )
        };
        status(code, "BCryptCreateHash")?;
        Ok(Self(handle))
    }

    fn update(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let len = u32::try_from(bytes.len()).map_err(std::io::Error::other)?;
        // SAFETY: the handle is live and `bytes` is readable for `len` bytes.
        status(
            unsafe { BCryptHashData(self.0, bytes.as_ptr(), len, 0) },
            "BCryptHashData",
        )
    }

    fn finish(self) -> std::io::Result<[u8; 32]> {
        let mut digest = [0_u8; 32];
        // SAFETY: the handle is live and `digest` is writable for its 32
        // bytes, the size of a SHA-256.
        let code = unsafe { BCryptFinishHash(self.0, digest.as_mut_ptr(), 32, 0) };
        status(code, "BCryptFinishHash")?;
        Ok(digest)
    }
}

impl Drop for Sha256 {
    fn drop(&mut self) {
        // SAFETY: the handle came from BCryptCreateHash and is destroyed once.
        unsafe { BCryptDestroyHash(self.0) };
    }
}

/// The attributes of `path`.
fn attributes(path: &Path) -> std::io::Result<u32> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call.
    let attrs = unsafe { GetFileAttributesW(wide.as_ptr()) };
    if attrs == INVALID_FILE_ATTRIBUTES {
        return Err(std::io::Error::last_os_error());
    }
    Ok(attrs)
}

/// Hashes the file at `path`. `cancelled` is asked before every buffer;
/// `progress` gets the bytes read so far after every buffer.
///
/// # Errors
///
/// The error from reading the attributes, opening or reading the file, or
/// from `BCrypt`.
pub fn sha256_file(
    path: &Path,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64),
) -> std::io::Result<Outcome> {
    if is_placeholder(attributes(path)?) {
        return Ok(Outcome::Placeholder);
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_SEQUENTIAL_SCAN | FILE_FLAG_OPEN_NO_RECALL)
        .open(path)?;
    let mut hash = Sha256::new()?;
    let mut buffer = vec![0_u8; CHUNK];
    let mut total: u64 = 0;
    loop {
        if cancelled() {
            return Ok(Outcome::Cancelled);
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(buffer.get(..read).unwrap_or_default())?;
        total = total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        progress(total);
    }
    hash.finish().map(Outcome::Done)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::PathBuf;

    use peekbuddy_core::filehash::{CHUNK, Outcome, hex};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_OFFLINE, SetFileAttributesW,
    };

    use super::sha256_file;

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("pb-hash-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn hash(path: &std::path::Path) -> Outcome {
        sha256_file(path, &|| false, &|_| {}).unwrap()
    }

    #[test]
    fn known_contents_give_the_published_digests() {
        let dir = Dir::new("known");
        let empty = dir.0.join("empty");
        std::fs::write(&empty, b"").unwrap();
        let abc = dir.0.join("abc");
        std::fs::write(&abc, b"abc").unwrap();
        let Outcome::Done(digest) = hash(&empty) else {
            unreachable!("an empty file is hashed")
        };
        assert_eq!(
            hex(&digest),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let Outcome::Done(digest) = hash(&abc) else {
            unreachable!("a small file is hashed")
        };
        assert_eq!(
            hex(&digest),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_file_larger_than_the_buffer_is_read_in_steps_and_reported() {
        let dir = Dir::new("steps");
        let path = dir.0.join("big");
        // A million 'a': the digest is the FIPS 180-2 test vector.
        std::fs::write(&path, vec![b'a'; 1_000_000]).unwrap();
        let steps = Cell::new(0_u32);
        let last = Cell::new(0_u64);
        let outcome = sha256_file(&path, &|| false, &|done| {
            steps.set(steps.get() + 1);
            last.set(done);
        })
        .unwrap();
        let Outcome::Done(digest) = outcome else {
            unreachable!("the file is hashed")
        };
        assert_eq!(
            hex(&digest),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
        assert_eq!(last.get(), 1_000_000);
        assert_eq!(
            steps.get(),
            1_000_000_u32.div_ceil(u32::try_from(CHUNK).unwrap())
        );
    }

    #[test]
    fn a_cancel_stops_before_the_next_buffer() {
        let dir = Dir::new("cancel");
        let path = dir.0.join("big");
        std::fs::write(&path, vec![0_u8; 3 * CHUNK]).unwrap();
        let read = Cell::new(0_u64);
        let outcome = sha256_file(&path, &|| read.get() >= CHUNK as u64, &|done| {
            read.set(done);
        })
        .unwrap();
        assert_eq!(outcome, Outcome::Cancelled);
        assert_eq!(read.get(), CHUNK as u64);
    }

    #[test]
    fn an_offline_file_is_refused_unread_and_a_missing_one_is_an_error() {
        let dir = Dir::new("offline");
        let path = dir.0.join("offline");
        std::fs::write(&path, b"in the cloud").unwrap();
        let wide: Vec<u16> = path
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call.
        let set = unsafe { SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_OFFLINE) };
        assert_ne!(set, 0);
        let read = Cell::new(false);
        let outcome = sha256_file(&path, &|| false, &|_| read.set(true)).unwrap();
        assert_eq!(outcome, Outcome::Placeholder);
        assert!(!read.get());
        // SAFETY: as above.
        unsafe { SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_NORMAL) };
        assert!(sha256_file(&dir.0.join("missing"), &|| false, &|_| {}).is_err());
    }

    #[test]
    fn a_file_open_for_writing_elsewhere_is_still_hashed() {
        let dir = Dir::new("shared");
        let path = dir.0.join("open");
        std::fs::write(&path, b"abc").unwrap();
        let _writer = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        assert!(matches!(hash(&path), Outcome::Done(_)));
    }
}
