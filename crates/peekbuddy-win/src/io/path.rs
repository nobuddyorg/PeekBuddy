//! Long paths and path classes (#43), without touching the OS.
//!
//! [`to_long_path`] builds the `\\?\` form that Win32 accepts beyond 260
//! units, from an absolute path as the user wrote it. Drive-relative input
//! (`C:foo`) is rejected: it depends on a per-drive current directory that a
//! watcher must never guess. Lone surrogates pass through untouched (F17).

use std::fmt;

const BACKSLASH: u16 = b'\\' as u16;
const SLASH: u16 = b'/' as u16;
const QUESTION: u16 = b'?' as u16;
const DOT: u16 = b'.' as u16;
const COLON: u16 = b':' as u16;

/// Why a path is not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    /// The path is empty.
    Empty,
    /// The path contains a NUL.
    Nul,
    /// A relative path.
    NotAbsolute,
    /// `C:foo`: a drive with no root directory.
    DriveRelative,
    /// A UNC path without a server or a share.
    IncompleteUnc,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "the path is empty",
            Self::Nul => "the path contains a NUL",
            Self::NotAbsolute => "the path is not absolute",
            Self::DriveRelative => "the path is relative to a drive (C:foo)",
            Self::IncompleteUnc => "the UNC path has no server or no share",
        })
    }
}

impl std::error::Error for PathError {}

/// Where a path lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathClass {
    /// A drive letter (a mount folder is a path on a drive letter).
    Local,
    /// A UNC share.
    Remote,
    /// A device or volume path (`\\.\`, `\\?\Volume{...}`, other `\\?\`
    /// forms without a drive letter).
    Device,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn is_separator(unit: u16) -> bool {
    unit == BACKSLASH || unit == SLASH
}

fn is_drive_letter(unit: u16) -> bool {
    u8::try_from(unit).is_ok_and(|b| b.is_ascii_alphabetic())
}

/// `\\?\` or `\??\`, the forms that Win32 and NT take verbatim.
fn has_device_prefix(path: &[u16]) -> bool {
    matches!(
        path,
        [BACKSLASH, BACKSLASH | QUESTION, QUESTION, BACKSLASH, ..]
    )
}

fn is_dot_device(path: &[u16]) -> bool {
    matches!(path, [BACKSLASH, BACKSLASH, DOT, BACKSLASH, ..])
}

/// The `\\?\` form of an absolute path.
///
/// - `C:\a\b` and `C:/a/b` give `\\?\C:\a\b`.
/// - `\\server\share\a` gives `\\?\UNC\server\share\a`.
/// - A path that already starts with `\\?\` or `\??\` is kept (it is
///   verbatim by definition).
///
/// Slashes become backslashes, because the `\\?\` form does no
/// normalisation. A trailing separator is kept.
///
/// # Errors
///
/// A [`PathError`] for an empty, relative, drive-relative or incomplete path,
/// or one with a NUL.
pub fn to_long_path(path: &[u16]) -> Result<Vec<u16>, PathError> {
    if path.is_empty() {
        return Err(PathError::Empty);
    }
    if path.contains(&0) {
        return Err(PathError::Nul);
    }
    if has_device_prefix(path) {
        return Ok(path.to_vec());
    }
    let normalised = |rest: &[u16]| -> Vec<u16> {
        rest.iter()
            .map(|&u| if u == SLASH { BACKSLASH } else { u })
            .collect()
    };
    // `\\server\share[\rest]`
    if let [first, second, rest @ ..] = path
        && is_separator(*first)
        && is_separator(*second)
    {
        let mut parts = rest.split(|&u| is_separator(u));
        let server = parts.next().unwrap_or_default();
        let share = parts.next().unwrap_or_default();
        if server.is_empty() || share.is_empty() {
            return Err(PathError::IncompleteUnc);
        }
        let mut out = wide(r"\\?\UNC\");
        out.extend(normalised(rest));
        return Ok(out);
    }
    // `C:\...`
    if let [drive, colon, rest @ ..] = path
        && is_drive_letter(*drive)
        && *colon == COLON
    {
        return match rest.first() {
            Some(&u) if is_separator(u) => {
                let mut out = wide(r"\\?\");
                out.extend(normalised(path));
                Ok(out)
            }
            _ => Err(PathError::DriveRelative),
        };
    }
    Err(PathError::NotAbsolute)
}

/// The class of a path, from its text alone.
#[must_use]
pub fn classify(path: &[u16]) -> PathClass {
    let unc = wide(r"\\?\UNC\");
    if path.len() > unc.len()
        && path
            .get(..unc.len())
            .is_some_and(|p| p.eq_ignore_ascii_case_units(&unc))
    {
        return PathClass::Remote;
    }
    if has_device_prefix(path) {
        // `\\?\C:\...` is a local drive path in device form.
        return match path.get(4..) {
            Some([d, COLON, ..]) if is_drive_letter(*d) => PathClass::Local,
            _ => PathClass::Device,
        };
    }
    if is_dot_device(path) {
        return PathClass::Device;
    }
    if let [first, second, ..] = path
        && is_separator(*first)
        && is_separator(*second)
    {
        return PathClass::Remote;
    }
    PathClass::Local
}

trait EqIgnoreAsciiUnits {
    fn eq_ignore_ascii_case_units(&self, other: &[u16]) -> bool;
}

impl EqIgnoreAsciiUnits for [u16] {
    fn eq_ignore_ascii_case_units(&self, other: &[u16]) -> bool {
        self.len() == other.len()
            && self.iter().zip(other).all(|(&a, &b)| {
                a == b
                    || match (u8::try_from(a), u8::try_from(b)) {
                        (Ok(x), Ok(y)) => x.eq_ignore_ascii_case(&y),
                        _ => false,
                    }
            })
    }
}

/// `path` without trailing separators, keeping the root of a drive (`C:\`)
/// and of a device path.
#[must_use]
pub fn trim_trailing_separators(path: &[u16]) -> &[u16] {
    let end = path
        .iter()
        .rposition(|&unit| !is_separator(unit))
        .map_or(0, |last| last + 1);
    // Keep `C:\`.
    if end == 2 && path.get(1) == Some(&COLON) {
        return path.get(..3).unwrap_or(path);
    }
    // Keep `\\?\C:\`.
    if has_device_prefix(path) && end == 6 && path.get(5) == Some(&COLON) {
        return path.get(..7).unwrap_or(path);
    }
    path.get(..end).unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long(text: &str) -> Result<String, PathError> {
        to_long_path(&wide(text)).map(|p| String::from_utf16_lossy(&p))
    }

    #[test]
    fn drive_paths_get_the_verbatim_prefix() {
        assert_eq!(long(r"C:\a\b").unwrap(), r"\\?\C:\a\b");
        assert_eq!(long("C:/a/b").unwrap(), r"\\?\C:\a\b");
        assert_eq!(long(r"c:\").unwrap(), r"\\?\c:\");
        assert_eq!(long(r"C:\a\b\").unwrap(), r"\\?\C:\a\b\");
    }

    #[test]
    fn unc_paths_get_the_unc_prefix() {
        assert_eq!(long(r"\\srv\share\a").unwrap(), r"\\?\UNC\srv\share\a");
        assert_eq!(long("//srv/share/a").unwrap(), r"\\?\UNC\srv\share\a");
        assert_eq!(long(r"\\srv\share").unwrap(), r"\\?\UNC\srv\share");
    }

    #[test]
    fn device_forms_are_kept() {
        assert_eq!(long(r"\\?\C:\a").unwrap(), r"\\?\C:\a");
        assert_eq!(long(r"\??\C:\a").unwrap(), r"\??\C:\a");
        assert_eq!(long(r"\\?\UNC\srv\share").unwrap(), r"\\?\UNC\srv\share");
        assert_eq!(long(r"\\?\Volume{1234}\a").unwrap(), r"\\?\Volume{1234}\a");
    }

    #[test]
    fn bad_paths_are_rejected() {
        assert_eq!(long(""), Err(PathError::Empty));
        assert_eq!(long("C:\\a\0b"), Err(PathError::Nul));
        assert_eq!(long("C:foo"), Err(PathError::DriveRelative));
        assert_eq!(long("C:"), Err(PathError::DriveRelative));
        assert_eq!(long(r"\\srv"), Err(PathError::IncompleteUnc));
        assert_eq!(long(r"\\srv\"), Err(PathError::IncompleteUnc));
        assert_eq!(long(r"\\\share"), Err(PathError::IncompleteUnc));
        assert_eq!(long(r"a\b"), Err(PathError::NotAbsolute));
        assert_eq!(long(r"\a\b"), Err(PathError::NotAbsolute));
        assert_eq!(long("1:\\a"), Err(PathError::NotAbsolute));
        assert_eq!(long(r"..\a"), Err(PathError::NotAbsolute));
        for error in [
            PathError::Empty,
            PathError::Nul,
            PathError::NotAbsolute,
            PathError::DriveRelative,
            PathError::IncompleteUnc,
        ] {
            assert!(error.to_string().starts_with("the "));
        }
    }

    #[test]
    fn long_paths_past_260_units_work() {
        let deep = format!(r"C:\{}", ["segment"; 60].join(r"\"));
        assert!(deep.len() > 260);
        let out = long(&deep).unwrap();
        assert_eq!(out.len(), deep.len() + 4);
        assert!(out.starts_with(r"\\?\C:\segment"));
    }

    #[test]
    fn lone_surrogates_pass_through() {
        let mut path = wide(r"C:\a\");
        path.push(0xD800);
        let out = to_long_path(&path).unwrap();
        assert_eq!(out.last(), Some(&0xD800));
        assert_eq!(out.len(), path.len() + 4);
    }

    #[test]
    fn classes() {
        let class = |t: &str| classify(&wide(t));
        assert_eq!(class(r"C:\a"), PathClass::Local);
        assert_eq!(class(r"C:\mnt\mounted-volume"), PathClass::Local);
        assert_eq!(class(r"\\?\C:\a"), PathClass::Local);
        assert_eq!(class(r"\\srv\share"), PathClass::Remote);
        assert_eq!(class(r"\\?\UNC\srv\share"), PathClass::Remote);
        assert_eq!(class(r"\\?\unc\srv\share"), PathClass::Remote);
        assert_eq!(class(r"\\?\Volume{1234}\"), PathClass::Device);
        assert_eq!(class(r"\??\Volume{1234}\"), PathClass::Device);
        assert_eq!(class(r"\\.\PhysicalDrive0"), PathClass::Device);
        assert_eq!(class(r"\\?\UNC\"), PathClass::Device);
        assert_eq!(class(r"\\?\UNC"), PathClass::Device);
        assert_eq!(class(r"\\?\1:\a"), PathClass::Device);
    }

    #[test]
    fn trailing_separators_go_but_roots_stay() {
        let trim = |t: &str| String::from_utf16_lossy(trim_trailing_separators(&wide(t)));
        assert_eq!(trim(r"C:\a\b\"), r"C:\a\b");
        assert_eq!(trim(r"C:\a\b\\"), r"C:\a\b");
        assert_eq!(trim("C:/a/b/"), "C:/a/b");
        assert_eq!(trim(r"C:\a\b"), r"C:\a\b");
        assert_eq!(trim(r"C:\"), r"C:\");
        assert_eq!(trim(r"C:\\"), r"C:\");
        assert_eq!(trim(r"\\?\C:\"), r"\\?\C:\");
        assert_eq!(trim(r"\\?\C:\a\"), r"\\?\C:\a");
        assert_eq!(trim(r"\\srv\share\"), r"\\srv\share");
        assert_eq!(trim(""), "");
        assert_eq!(trim(r"\"), "");
        assert_eq!(trim("C:"), "C:");
        assert_eq!(trim(r"\\?\C:"), r"\\?\C:");
    }
}
