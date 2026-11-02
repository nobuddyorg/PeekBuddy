//! Link targets as watches (#265, F18): for a watch with `follow_link_target`
//! the tree is searched for directory links (symlinks and junctions) and
//! every target that is not watched yet becomes a watch of its own.
//!
//! The search runs once at start, over the real directories of the tree only
//! (a link is never entered, a target is searched for links of its own). A
//! link created later is picked up at the next start. The added watches are
//! marked [`Watch::derived_from`], so they show up in the tray and can be paused,
//! and are never written to `settings.toml`.

use std::os::windows::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

use peekbuddy_core::reparse::follow::{Admit, LinkTargets, Report};
use peekbuddy_core::settings::{Settings, Watch};

/// The most targets added for one start; a tree with more links than this is
/// not one the user means to follow.
pub const MAX_TARGETS: usize = 256;

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().collect()
}

/// The path without the `\\?\` prefix `canonicalize` adds.
fn plain(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned())
}

/// Adds a derived watch for every link target below the watches that ask for
/// it. Returns what was seen.
pub fn follow_link_targets(settings: &mut Settings) -> Report {
    let mut targets = LinkTargets::new();
    for watch in &settings.watches {
        let path = Path::new(&watch.path);
        if let Ok(identity) = peekbuddy_win::fs::file_identity(path) {
            targets.claim(identity, &wide(path));
        }
    }
    let origins: Vec<Watch> = settings
        .watches
        .iter()
        .filter(|watch| watch.follow_link_target && watch.enabled && watch.recursive)
        .cloned()
        .collect();
    let mut added: Vec<Watch> = Vec::new();
    for origin in &origins {
        let mut queue = vec![PathBuf::from(&origin.path)];
        let mut count = 0_u32;
        while let Some(dir) = queue.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    queue.push(entry.path());
                } else if kind.is_symlink() {
                    let Some(target) = resolve(&entry.path(), &mut targets) else {
                        continue;
                    };
                    if added.len() >= MAX_TARGETS {
                        continue;
                    }
                    count = count.saturating_add(1);
                    queue.push(target.clone());
                    added.push(Watch {
                        id: format!("{}.link{count}", origin.id),
                        path: target.to_string_lossy().into_owned(),
                        derived_from: Some(origin.id.clone()),
                        follow_link_target: false,
                        ..origin.clone()
                    });
                }
            }
        }
    }
    settings.watches.extend(added);
    targets.report()
}

/// The target of the directory link `link` when it is to be watched.
fn resolve(link: &Path, targets: &mut LinkTargets) -> Option<PathBuf> {
    let Ok(metadata) = std::fs::metadata(link) else {
        targets.unreadable();
        return None;
    };
    if !metadata.is_dir() {
        return None;
    }
    let canonical = std::fs::canonicalize(link).ok()?;
    let Ok(identity) = peekbuddy_win::fs::file_identity(&canonical) else {
        targets.unreadable();
        return None;
    };
    (targets.admit(identity, &wide(&canonical)) == Admit::New).then(|| plain(&canonical))
}

/// The line the engine log gets for a search.
#[must_use]
pub fn describe(report: Report) -> String {
    format!(
        "links: followed {}, duplicate {}, overlapping {}, unreadable {}",
        report.followed, report.duplicates, report.overlapping, report.unreadable
    )
}
