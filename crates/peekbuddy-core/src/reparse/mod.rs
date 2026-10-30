//! Reparse points and cloud placeholders (#49).
//!
//! Two pure decisions live here:
//!
//! - What a reparse tag is ([`link_kind`]), so the link object itself can be
//!   logged with a classification and never followed out of a watched root.
//! - Whether a change to a cloud placeholder is sync churn ([`classify`]):
//!   `OneDrive` and its kin flip attributes and rewrite the reparse data all
//!   day, and the user does not want those rows in the default view. Churn is
//!   tagged (`Flags::CLOUD_CHURN`) and hidden in the view, never dropped (raw
//!   is truth).
//!
//! A hydrated `OneDrive` file is still a reparse point, so "is a reparse point"
//! never decides anything: the rules key on the reason set, the source and
//! the attribute delta, and a record that creates, deletes, renames, changes
//! security or hard links, or carries a user data write, is never churn.
//!
//! The rules are code, written from the issue; the lab matrix of #36 could not
//! measure `OneDrive` on a hosted runner (spike 0006, "Not reproducible"), so
//! they are the issue's rules and nothing more, and the `OneDrive` cells need
//! an owner run before the rule set is widened.

use crate::event::map::usn_reason as reason;

/// Reparse tags.
pub mod tag {
    /// `IO_REPARSE_TAG_MOUNT_POINT`: a junction or volume mount point.
    pub const MOUNT_POINT: u32 = 0xA000_0003;
    /// `IO_REPARSE_TAG_SYMLINK`.
    pub const SYMLINK: u32 = 0xA000_000C;
    /// `IO_REPARSE_TAG_LX_SYMLINK`: a WSL symlink.
    pub const LX_SYMLINK: u32 = 0xA000_001D;
    /// `IO_REPARSE_TAG_APPEXECLINK`.
    pub const APPEXECLINK: u32 = 0x8000_001B;
    /// `IO_REPARSE_TAG_PROJFS`.
    pub const PROJFS: u32 = 0x9000_001C;
    /// `IO_REPARSE_TAG_CLOUD`: the base of the cloud placeholder tags; bits
    /// 12 to 15 carry a state.
    pub const CLOUD: u32 = 0x9000_001A;
    /// Mask that clears the state nibble of a cloud tag.
    pub const CLOUD_MASK: u32 = 0xFFFF_0FFF;
}

/// `FILE_ATTRIBUTE_*` bits that matter for placeholders.
pub mod attr {
    /// `FILE_ATTRIBUTE_OFFLINE`.
    pub const OFFLINE: u32 = 0x0000_1000;
    /// `FILE_ATTRIBUTE_SPARSE_FILE`.
    pub const SPARSE: u32 = 0x0000_0200;
    /// `FILE_ATTRIBUTE_PINNED`.
    pub const PINNED: u32 = 0x0008_0000;
    /// `FILE_ATTRIBUTE_UNPINNED`.
    pub const UNPINNED: u32 = 0x0010_0000;
    /// `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`: the data is not local.
    pub const RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
    /// The attributes a sync client flips without the user touching the file.
    pub const SYNC_STATE: u32 = OFFLINE | SPARSE | PINNED | UNPINNED | RECALL_ON_DATA_ACCESS;
}

/// `USN_SOURCE_CLIENT_REPLICATION_MANAGEMENT`.
pub const SOURCE_CLIENT_REPLICATION_MANAGEMENT: u32 = 0x8;

/// What kind of object a reparse tag marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// A junction or mount point.
    MountPoint,
    /// A symbolic link.
    Symlink,
    /// A WSL symlink.
    WslSymlink,
    /// An app-execution alias (never resolved, never followed).
    AppExecLink,
    /// A projected-file-system placeholder.
    ProjFs,
    /// A cloud placeholder (`OneDrive` and others).
    CloudPlaceholder,
    /// Any other tag.
    Other,
}

impl LinkKind {
    /// Whether the object is a link that traversal must not follow. Cloud and
    /// projected placeholders are files, not links.
    #[must_use]
    pub const fn is_link(self) -> bool {
        matches!(
            self,
            Self::MountPoint | Self::Symlink | Self::WslSymlink | Self::AppExecLink
        )
    }
}

/// Whether `tag` is a cloud placeholder tag (any state nibble).
#[must_use]
pub const fn is_cloud(tag: u32) -> bool {
    tag & tag::CLOUD_MASK == tag::CLOUD
}

/// The kind a reparse tag stands for; `None` for tag 0 (not a reparse point).
#[must_use]
pub const fn link_kind(tag: u32) -> Option<LinkKind> {
    Some(match tag {
        0 => return None,
        tag::MOUNT_POINT => LinkKind::MountPoint,
        tag::SYMLINK => LinkKind::Symlink,
        tag::LX_SYMLINK => LinkKind::WslSymlink,
        tag::APPEXECLINK => LinkKind::AppExecLink,
        tag::PROJFS => LinkKind::ProjFs,
        other if is_cloud(other) => LinkKind::CloudPlaceholder,
        _ => LinkKind::Other,
    })
}

/// What a churn row stands for, derived from the attribute and tag delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    /// The data became local.
    Hydrated,
    /// The data went back to the cloud.
    Dehydrated,
    /// Pinned: kept on this device.
    Pinned,
    /// Unpinned: may be freed.
    Unpinned,
    /// Nothing visible changed; the client confirmed the file is in sync.
    InSync,
    /// A normal file became a placeholder.
    Converted,
}

/// What the classifier looks at, from one record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    /// The reparse tag after the change (0 when not a reparse point).
    pub tag: u32,
    /// The reparse tag before, when known.
    pub tag_before: Option<u32>,
    /// `USN_REASON_*` bits, or 0 for a source without them.
    pub reason: u32,
    /// `USN_SOURCE_*` bits.
    pub source_info: u32,
    /// Attributes before the change, when known.
    pub attrs_before: Option<u32>,
    /// Attributes after the change.
    pub attrs_after: u32,
}

/// Reasons that are never churn, whatever else holds.
const NEVER: u32 = reason::FILE_CREATE
    | reason::FILE_DELETE
    | reason::RENAME_OLD_NAME
    | reason::RENAME_NEW_NAME
    | reason::SECURITY_CHANGE
    | reason::HARD_LINK_CHANGE;

/// Reasons a sync client may produce without the user doing anything.
const STATE_ONLY: u32 = reason::BASIC_INFO_CHANGE | reason::REPARSE_POINT_CHANGE | reason::CLOSE;

/// Whether the change is cloud-state churn, and what it stands for.
#[must_use]
pub fn classify(change: &Change) -> Option<Label> {
    if !is_cloud(change.tag) || change.reason & NEVER != 0 {
        return None;
    }
    let data = change.reason & reason::DATA != 0;
    let replicated = change.source_info & SOURCE_CLIENT_REPLICATION_MANAGEMENT != 0;
    if data && !replicated {
        // A user data write.
        return None;
    }
    let state_only = change.reason != 0 && change.reason & !STATE_ONLY == 0;
    let flipped = change.attrs_before.is_some_and(|before| {
        (before ^ change.attrs_after) & !attr::SYNC_STATE == 0 && before != change.attrs_after
    });
    if !(state_only || (data && replicated) || flipped) {
        return None;
    }
    Some(label_of(change))
}

fn label_of(change: &Change) -> Label {
    if change.tag_before.is_some_and(|before| !is_cloud(before)) {
        return Label::Converted;
    }
    let before = change.attrs_before.unwrap_or(change.attrs_after);
    let after = change.attrs_after;
    let gained = after & !before;
    let lost = before & !after;
    if lost & attr::RECALL_ON_DATA_ACCESS != 0 {
        Label::Hydrated
    } else if gained & attr::RECALL_ON_DATA_ACCESS != 0 {
        Label::Dehydrated
    } else if gained & attr::PINNED != 0 {
        Label::Pinned
    } else if gained & attr::UNPINNED != 0 {
        Label::Unpinned
    } else {
        Label::InSync
    }
}

#[cfg(test)]
mod tests;
