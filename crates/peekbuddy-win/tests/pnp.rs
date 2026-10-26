//! The plug-and-play and power registrations against the OS (#48).
#![cfg(windows)]

use std::os::windows::ffi::OsStrExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use peekbuddy_win::io::path::to_long_path;
use peekbuddy_win::pnp::{
    HandleWatch, InterfaceWatch, Notice, PowerWatch, notice_of_action, notice_of_event,
};
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_NOTIFY_ACTION_DEVICEQUERYREMOVE, CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED,
    CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE, CM_NOTIFY_ACTION_DEVICEREMOVEPENDING,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GUID_IO_VOLUME_DISMOUNT, GUID_IO_VOLUME_DISMOUNT_FAILED, GUID_IO_VOLUME_FVE_STATUS_CHANGE,
    GUID_IO_VOLUME_LOCK, GUID_IO_VOLUME_LOCK_FAILED, GUID_IO_VOLUME_MOUNT, GUID_IO_VOLUME_UNLOCK,
};
use windows_sys::core::GUID;

#[test]
fn custom_event_guids_map_to_notices() {
    let table = [
        (GUID_IO_VOLUME_LOCK, Notice::Lock),
        (GUID_IO_VOLUME_LOCK_FAILED, Notice::LockFailed),
        (GUID_IO_VOLUME_UNLOCK, Notice::Unlock),
        (GUID_IO_VOLUME_DISMOUNT, Notice::Dismount),
        (GUID_IO_VOLUME_DISMOUNT_FAILED, Notice::DismountFailed),
        (GUID_IO_VOLUME_MOUNT, Notice::Mount),
        (GUID_IO_VOLUME_FVE_STATUS_CHANGE, Notice::FveChange),
    ];
    for (guid, notice) in table {
        assert_eq!(notice_of_event(&guid), Some(notice));
    }
    assert_eq!(notice_of_event(&GUID::from_u128(1)), None);
}

#[test]
fn plain_actions_map_to_notices() {
    assert_eq!(
        notice_of_action(CM_NOTIFY_ACTION_DEVICEQUERYREMOVE),
        Some(Notice::QueryRemove)
    );
    assert_eq!(
        notice_of_action(CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED),
        Some(Notice::QueryRemoveFailed)
    );
    assert_eq!(
        notice_of_action(CM_NOTIFY_ACTION_DEVICEREMOVEPENDING),
        Some(Notice::RemovePending)
    );
    assert_eq!(
        notice_of_action(CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE),
        Some(Notice::RemoveComplete)
    );
    assert_eq!(notice_of_action(99), None);
}

#[test]
fn registrations_can_be_made_and_dropped_repeatedly() {
    let dir = std::env::temp_dir();
    let wide: Vec<u16> = dir.as_os_str().encode_wide().collect();
    let long = to_long_path(&wide).unwrap();
    let seen = Arc::new(AtomicUsize::new(0));
    for _ in 0..5 {
        let counter = Arc::clone(&seen);
        let handle = HandleWatch::register(
            &long,
            Arc::new(move |_| {
                counter.fetch_add(1, Ordering::Relaxed);
            }),
        )
        .unwrap();
        let interfaces = InterfaceWatch::register(Arc::new(|_| {})).unwrap();
        let power = PowerWatch::register(Arc::new(|_| {})).unwrap();
        drop((handle, interfaces, power));
    }
    // Nothing happened to the volume of the temp directory.
    assert_eq!(seen.load(Ordering::Relaxed), 0);
}
