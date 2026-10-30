use proptest::prelude::*;

use super::*;

const CLOUD: u32 = tag::CLOUD;
const PLAIN: u32 = 0x20;

fn change(tag: u32, reason: u32, source_info: u32, before: u32, after: u32) -> Change {
    Change {
        tag,
        tag_before: Some(tag),
        reason,
        source_info,
        attrs_before: Some(before),
        attrs_after: after,
    }
}

#[test]
fn tags_map_to_kinds() {
    let table = [
        (0, None),
        (tag::MOUNT_POINT, Some(LinkKind::MountPoint)),
        (tag::SYMLINK, Some(LinkKind::Symlink)),
        (tag::LX_SYMLINK, Some(LinkKind::WslSymlink)),
        (tag::APPEXECLINK, Some(LinkKind::AppExecLink)),
        (tag::PROJFS, Some(LinkKind::ProjFs)),
        (0x9000_001A, Some(LinkKind::CloudPlaceholder)),
        (0x9000_601A, Some(LinkKind::CloudPlaceholder)),
        (0x9000_F01A, Some(LinkKind::CloudPlaceholder)),
        (0x8000_0017, Some(LinkKind::Other)),
    ];
    for (value, kind) in table {
        assert_eq!(link_kind(value), kind, "{value:#x}");
    }
}

#[test]
fn only_real_links_are_followed_candidates() {
    assert!(LinkKind::MountPoint.is_link());
    assert!(LinkKind::AppExecLink.is_link());
    assert!(!LinkKind::CloudPlaceholder.is_link());
    assert!(!LinkKind::ProjFs.is_link());
    assert!(!LinkKind::Other.is_link());
}

#[test]
fn a_basic_info_change_on_a_placeholder_is_churn() {
    let c = change(
        CLOUD,
        reason::BASIC_INFO_CHANGE | reason::CLOSE,
        0,
        PLAIN,
        PLAIN,
    );
    assert_eq!(classify(&c), Some(Label::InSync));
    let c = change(CLOUD, reason::REPARSE_POINT_CHANGE, 0, PLAIN, PLAIN);
    assert_eq!(classify(&c), Some(Label::InSync));
}

#[test]
fn the_same_reasons_on_a_plain_file_are_not_churn() {
    let c = change(
        0,
        reason::BASIC_INFO_CHANGE | reason::CLOSE,
        0,
        PLAIN,
        PLAIN,
    );
    assert_eq!(classify(&c), None);
}

#[test]
fn data_written_by_the_sync_client_is_churn_but_a_user_write_is_not() {
    let by_client = change(
        CLOUD,
        reason::DATA_EXTEND | reason::CLOSE,
        SOURCE_CLIENT_REPLICATION_MANAGEMENT,
        PLAIN,
        PLAIN,
    );
    assert!(classify(&by_client).is_some());
    let by_user = change(CLOUD, reason::DATA_EXTEND | reason::CLOSE, 0, PLAIN, PLAIN);
    assert_eq!(classify(&by_user), None);
}

#[test]
fn an_attribute_flip_of_the_sync_state_is_churn_with_a_label() {
    let dehydrated = change(
        CLOUD,
        0,
        0,
        PLAIN,
        PLAIN | attr::RECALL_ON_DATA_ACCESS | attr::OFFLINE,
    );
    assert_eq!(classify(&dehydrated), Some(Label::Dehydrated));
    let hydrated = change(CLOUD, 0, 0, PLAIN | attr::RECALL_ON_DATA_ACCESS, PLAIN);
    assert_eq!(classify(&hydrated), Some(Label::Hydrated));
    let pinned = change(CLOUD, 0, 0, PLAIN, PLAIN | attr::PINNED);
    assert_eq!(classify(&pinned), Some(Label::Pinned));
    let unpinned = change(CLOUD, 0, 0, PLAIN, PLAIN | attr::UNPINNED);
    assert_eq!(classify(&unpinned), Some(Label::Unpinned));
}

#[test]
fn a_flip_with_another_attribute_change_is_not_churn() {
    // The read-only bit changed too: the user did something.
    let c = change(CLOUD, 0, 0, PLAIN, PLAIN | attr::PINNED | 0x1);
    assert_eq!(classify(&c), None);
}

#[test]
fn a_file_that_becomes_a_placeholder_is_converted() {
    let mut c = change(
        CLOUD,
        reason::REPARSE_POINT_CHANGE,
        0,
        PLAIN,
        PLAIN | attr::RECALL_ON_DATA_ACCESS,
    );
    c.tag_before = Some(0);
    assert_eq!(classify(&c), Some(Label::Converted));
}

#[test]
fn the_never_churn_reasons_win_over_every_rule() {
    for never in [
        reason::FILE_CREATE,
        reason::FILE_DELETE,
        reason::RENAME_OLD_NAME,
        reason::RENAME_NEW_NAME,
        reason::SECURITY_CHANGE,
        reason::HARD_LINK_CHANGE,
    ] {
        let c = change(
            CLOUD,
            never | reason::BASIC_INFO_CHANGE,
            SOURCE_CLIENT_REPLICATION_MANAGEMENT,
            PLAIN,
            PLAIN | attr::PINNED,
        );
        assert_eq!(classify(&c), None, "{never:#x}");
    }
}

proptest! {
    #[test]
    fn create_delete_rename_security_and_link_records_are_never_churn(
        tag in prop_oneof![Just(CLOUD), 0x9000_001A_u32..=0x9000_F01A, any::<u32>()],
        reasons in any::<u32>(),
        source in any::<u32>(),
        before in proptest::option::of(any::<u32>()),
        after in any::<u32>(),
        pick in 0usize..6,
    ) {
        let forced = [
            reason::FILE_CREATE,
            reason::FILE_DELETE,
            reason::RENAME_OLD_NAME,
            reason::RENAME_NEW_NAME,
            reason::SECURITY_CHANGE,
            reason::HARD_LINK_CHANGE,
        ];
        let c = Change {
            tag,
            tag_before: None,
            reason: reasons | forced.get(pick).copied().unwrap_or(0),
            source_info: source,
            attrs_before: before,
            attrs_after: after,
        };
        prop_assert_eq!(classify(&c), None);
    }

    #[test]
    fn a_user_data_write_is_never_churn(
        reasons in any::<u32>(),
        before in proptest::option::of(any::<u32>()),
        after in any::<u32>(),
    ) {
        let c = Change {
            tag: CLOUD,
            tag_before: None,
            reason: reasons | reason::DATA_OVERWRITE,
            source_info: 0,
            attrs_before: before,
            attrs_after: after,
        };
        prop_assert_eq!(classify(&c), None);
    }

    #[test]
    fn a_plain_file_is_never_churn(
        tag in any::<u32>().prop_filter("not a cloud tag", |t| !is_cloud(*t)),
        reasons in any::<u32>(),
        source in any::<u32>(),
        before in proptest::option::of(any::<u32>()),
        after in any::<u32>(),
    ) {
        let c = Change {
            tag,
            tag_before: None,
            reason: reasons,
            source_info: source,
            attrs_before: before,
            attrs_after: after,
        };
        prop_assert_eq!(classify(&c), None);
    }
}
