//! Adding and removing watches on behalf of the viewer (#68).
//!
//! The engine decides, the viewer only shows: what may be watched, what the
//! user has to confirm and what is only worth a word of information are all
//! decided here, from the settings and the machine's two refused directories.
//! The functions are pure, so they run on Linux and are tested there.

use super::validate::{Context, is_inside, is_unc, is_whole_drive, normalize_root};
use super::{Settings, Watch};

/// Something about a new watch the user should know. Notices never stop an
/// add, except [`Notice::WholeDrive`], which has to be confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// The root is a whole drive. Noisy, and memory grows with the number of
    /// watched directories (F8): the user confirms explicitly.
    WholeDrive,
    /// A network path: a 64 KiB buffer and a slow health check (F6).
    Network,
    /// The folder lies inside a recursive watch that already records it.
    CoveredBy {
        /// The id of the watch that covers it.
        id: String,
    },
    /// The folder contains watches that exist already; their events would
    /// be recorded once, the capture deduplicates by file identity.
    Covers {
        /// The ids of the contained watches.
        ids: Vec<String>,
    },
}

impl Notice {
    /// The sentence the viewer shows.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::WholeDrive => String::from(
                "This is a whole drive. It is noisy, and memory grows with the number of \
                 folders watched. Consider watching the folders you care about instead.",
            ),
            Self::Network => String::from(
                "This is a network folder: PeekBuddy uses a small change buffer (64 KiB) \
                 and checks its health about once a minute.",
            ),
            Self::CoveredBy { id } => {
                format!("This folder is already watched as part of the watch `{id}`.")
            }
            Self::Covers { ids } => format!(
                "This folder contains the watches {}. Each change is recorded once.",
                ids.iter()
                    .map(|id| format!("`{id}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// Why a watch cannot be added or removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The path is not usable at all.
    Invalid(String),
    /// The path is the data directory or inside it (ADR 0008).
    DataDir,
    /// The path is the export directory or inside it (F16).
    ExportDir,
    /// The root is watched already.
    Duplicate {
        /// The id of the watch with this root.
        id: String,
    },
    /// A whole drive needs the user's confirmation first.
    NeedsConfirmation(Notice),
    /// There is no watch with this id.
    Unknown(String),
    /// Too many watches for the id space to stay readable.
    TooMany,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => write!(f, "{message}"),
            Self::DataDir => write!(
                f,
                "That folder is PeekBuddy's own data folder (or inside it). Watching it \
                 would record PeekBuddy writing its own log."
            ),
            Self::ExportDir => write!(
                f,
                "That folder is the export folder (or inside it). Watching it would record \
                 PeekBuddy writing its own exports."
            ),
            Self::Duplicate { id } => write!(f, "That folder is already watched (`{id}`)."),
            Self::NeedsConfirmation(notice) => write!(f, "{}", notice.text()),
            Self::Unknown(id) => write!(f, "There is no watch `{id}`."),
            Self::TooMany => write!(f, "There are too many watches."),
        }
    }
}

impl std::error::Error for Refusal {}

/// The most watches one file may hold; also keeps generated ids unique with
/// a short counter.
pub const MAX_WATCHES: usize = 256;

/// What a successful add did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    /// The new watch, as stored.
    pub watch: Watch,
    /// Information for the user.
    pub notices: Vec<Notice>,
}

/// What a successful removal did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    /// The watch that was taken out.
    pub watch: Watch,
}

/// Checks `path` against `settings` and `ctx` without changing anything.
///
/// # Errors
///
/// The [`Refusal`]; a whole drive is refused with
/// [`Refusal::NeedsConfirmation`] unless `confirmed`.
pub fn check(
    settings: &Settings,
    path: &str,
    confirmed: bool,
    ctx: &Context,
) -> Result<(String, Vec<Notice>), Refusal> {
    let normalised = normalize_root(path.trim()).map_err(Refusal::Invalid)?;
    for (refusal, dir) in [
        (Refusal::DataDir, Some(&ctx.data_dir)),
        (Refusal::ExportDir, ctx.export_dir.as_ref()),
    ] {
        let dir = dir.and_then(|dir| normalize_root(dir).ok());
        if dir.is_some_and(|dir| is_inside(&normalised, &dir)) {
            return Err(refusal);
        }
    }
    if settings.watches.len() >= MAX_WATCHES {
        return Err(Refusal::TooMany);
    }
    let mut notices = Vec::new();
    let mut covered_by = None;
    let mut covers = Vec::new();
    for watch in &settings.watches {
        let Ok(existing) = normalize_root(&watch.path) else {
            continue;
        };
        if existing.eq_ignore_ascii_case(&normalised) {
            return Err(Refusal::Duplicate {
                id: watch.id.clone(),
            });
        }
        if watch.recursive && is_inside(&normalised, &existing) && covered_by.is_none() {
            covered_by = Some(watch.id.clone());
        }
        if is_inside(&existing, &normalised) {
            covers.push(watch.id.clone());
        }
    }
    if is_whole_drive(&normalised) {
        if !confirmed {
            return Err(Refusal::NeedsConfirmation(Notice::WholeDrive));
        }
        notices.push(Notice::WholeDrive);
    }
    if is_unc(&normalised) {
        notices.push(Notice::Network);
    }
    if let Some(id) = covered_by {
        notices.push(Notice::CoveredBy { id });
    }
    if !covers.is_empty() {
        notices.push(Notice::Covers { ids: covers });
    }
    Ok((normalised, notices))
}

/// Adds a recursive watch on `path`.
///
/// # Errors
///
/// See [`check`].
pub fn add_watch(
    settings: &mut Settings,
    path: &str,
    confirmed: bool,
    ctx: &Context,
) -> Result<Added, Refusal> {
    let (normalised, notices) = check(settings, path, confirmed, ctx)?;
    let watch = Watch {
        id: new_id(settings, &normalised),
        path: normalised,
        ..Watch::default()
    };
    settings.watches.push(watch.clone());
    Ok(Added { watch, notices })
}

/// Removes the watch `id`. Rows already recorded stay in the log.
///
/// # Errors
///
/// [`Refusal::Unknown`] when there is no such watch.
pub fn remove_watch(settings: &mut Settings, id: &str) -> Result<Removed, Refusal> {
    let index = settings
        .watches
        .iter()
        .position(|watch| watch.id == id && watch.derived_from.is_none())
        .ok_or_else(|| Refusal::Unknown(id.to_owned()))?;
    Ok(Removed {
        watch: settings.watches.remove(index),
    })
}

/// A stable id for a new root: the folder name in the id alphabet and eight
/// hex digits of the path's hash. The same folder gets the same id when it
/// is added again, so an id never means two different folders; a clash
/// between current watches gets a counter.
#[must_use]
pub fn new_id(settings: &Settings, normalised: &str) -> String {
    let leaf = normalised
        .trim_end_matches('\\')
        .rsplit('\\')
        .next()
        .unwrap_or_default();
    let mut slug: String = leaf
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                '-'
            }
        })
        .collect();
    // A drive root's leaf is `c:`.
    slug = slug.trim_matches('-').to_owned();
    if slug.is_empty() {
        slug.push_str("root");
    }
    slug.truncate(40);
    let base = format!(
        "{slug}-{:08x}",
        fnv1a(&normalised.to_lowercase()) & 0xffff_ffff
    );
    let taken = |id: &str| settings.watches.iter().any(|watch| watch.id == id);
    if !taken(&base) {
        return base;
    }
    (2..=MAX_WATCHES + 2)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken(id))
        .unwrap_or(base)
}

fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
#[expect(
    clippy::panic,
    clippy::indexing_slicing,
    reason = "tests may panic and index"
)]
mod tests;
