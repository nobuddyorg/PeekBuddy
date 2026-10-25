//! Crash log and marker.
//!
//! Release builds use `panic = "abort"`, so a panic hook is the only chance to
//! leave evidence: it writes `logs\crash-<utc>.log` (message, location,
//! thread, build, OS build, backtrace) and a marker file. No minidump and no
//! network. The next start finds the marker, records the abnormal
//! termination and runs the normal cold-start reconciliation.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Name of the marker file in the data directory.
pub const MARKER_FILE: &str = "crash.marker";

/// `YYYYMMDDTHHMMSSZ` for `time`, in UTC.
#[must_use]
pub fn utc_stamp(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let rest = seconds % 86_400;
    // Civil-from-days (Howard Hinnant), valid for the whole proleptic range.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// What the crash log says about the build and the machine.
#[derive(Debug, Clone)]
pub struct BuildInfo {
    /// Package version and, when known, the git SHA.
    pub build: String,
    /// `major.minor.build` of Windows.
    pub os_build: String,
}

/// Formats the crash log text.
#[must_use]
pub fn format_report(
    info: &BuildInfo,
    message: &str,
    location: &str,
    thread: &str,
    backtrace: &str,
) -> String {
    format!(
        "PeekBuddy crash report\nbuild: {}\nos: {}\nthread: {thread}\nlocation: {location}\nmessage: {message}\n\nbacktrace:\n{backtrace}\n",
        info.build, info.os_build
    )
}

/// Installs the panic hook. `logs` receives the crash log, `data_root` the
/// marker. The previous hook (the default one, which prints to a console that
/// the GUI exe does not have) still runs afterwards.
pub fn install_panic_hook(logs: PathBuf, data_root: PathBuf, info: BuildInfo) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let message = panic
            .payload()
            .downcast_ref::<&str>()
            .map(ToString::to_string)
            .or_else(|| panic.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| String::from("<non-string payload>"));
        let location = panic
            .location()
            .map_or_else(|| String::from("<unknown>"), ToString::to_string);
        let current = std::thread::current();
        let thread = current.name().unwrap_or("<unnamed>");
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let report = format_report(&info, &message, &location, thread, &backtrace);
        let name = format!("crash-{}.log", utc_stamp(SystemTime::now()));
        // Nothing can be reported from here if writing fails.
        let _ = std::fs::create_dir_all(&logs);
        if std::fs::write(logs.join(&name), report).is_ok() {
            let _ = std::fs::write(data_root.join(MARKER_FILE), name);
        }
        previous(panic);
    }));
}

/// Reads and removes the crash marker. Returns the name of the crash log it
/// points to when the previous run ended abnormally.
#[must_use]
pub fn take_marker(data_root: &Path) -> Option<String> {
    let path = data_root.join(MARKER_FILE);
    let log = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(path);
    Some(log)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{BuildInfo, MARKER_FILE, format_report, take_marker, utc_stamp};

    #[test]
    fn the_stamp_is_utc_and_sortable() {
        assert_eq!(utc_stamp(UNIX_EPOCH), "19700101T000000Z");
        // 2000-02-29 12:34:56, a leap day.
        let leap = UNIX_EPOCH + Duration::from_secs(951_827_696);
        assert_eq!(utc_stamp(leap), "20000229T123456Z");
        // 2026-10-25 21:18:14, the day of this change.
        let now = UNIX_EPOCH + Duration::from_secs(1_792_963_094);
        assert_eq!(utc_stamp(now), "20261025T211814Z");
    }

    #[test]
    fn the_report_names_what_a_bug_report_needs() {
        let info = BuildInfo {
            build: String::from("0.0.0+abc"),
            os_build: String::from("10.0.26100"),
        };
        let text = format_report(&info, "boom", "src/lib.rs:1:1", "main", "frames");
        for needle in [
            "0.0.0+abc",
            "10.0.26100",
            "boom",
            "src/lib.rs:1:1",
            "main",
            "frames",
        ] {
            assert!(text.contains(needle), "{needle} missing in {text}");
        }
    }

    #[test]
    fn a_marker_is_taken_exactly_once() {
        let dir = std::env::temp_dir().join(format!("pb-crash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(take_marker(&dir), None);
        std::fs::write(dir.join(MARKER_FILE), "crash-1.log").unwrap();
        assert_eq!(take_marker(&dir).as_deref(), Some("crash-1.log"));
        assert_eq!(take_marker(&dir), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
