//! Plug-and-play and power notifications (#48).
//!
//! Three registrations, each with a handler that must only post to a queue,
//! never block: the callbacks run on system threads, and a slow answer to a
//! query-remove makes "Safely Remove Hardware" fail.
//!
//! - [`HandleWatch`]: `CM_Register_Notification` on a handle to a directory
//!   of a volume. It reports the volume's query-remove, remove, lock,
//!   dismount, mount and `BitLocker` status changes ([`Notice`]). The handle is
//!   held by the registration, so a `QueryRemove` must be answered by
//!   dropping the [`HandleWatch`] (the engine does so when it closes the
//!   watch), never from inside the callback.
//! - [`InterfaceWatch`]: arrival and removal of volume device interfaces.
//! - [`PowerWatch`]: `PowerRegisterSuspendResumeNotification` in callback
//!   form, which is the form that works with Modern Standby.

use std::io;
use std::os::windows::io::AsRawHandle;
use std::sync::Arc;

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_NOTIFY_ACTION, CM_NOTIFY_ACTION_DEVICECUSTOMEVENT, CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL,
    CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL, CM_NOTIFY_ACTION_DEVICEQUERYREMOVE,
    CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED, CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE,
    CM_NOTIFY_ACTION_DEVICEREMOVEPENDING, CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER,
    CM_NOTIFY_FILTER_0_0, CM_NOTIFY_FILTER_0_1, CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE,
    CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, CM_Register_Notification, CM_Unregister_Notification,
    HCMNOTIFICATION,
};
use windows_sys::Win32::System::Ioctl::GUID_DEVINTERFACE_VOLUME;
use windows_sys::Win32::System::Power::{
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, PowerRegisterSuspendResumeNotification,
    PowerUnregisterSuspendResumeNotification,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DEVICE_NOTIFY_CALLBACK, GUID_IO_VOLUME_DISMOUNT, GUID_IO_VOLUME_DISMOUNT_FAILED,
    GUID_IO_VOLUME_FVE_STATUS_CHANGE, GUID_IO_VOLUME_LOCK, GUID_IO_VOLUME_LOCK_FAILED,
    GUID_IO_VOLUME_MOUNT, GUID_IO_VOLUME_UNLOCK,
};
use windows_sys::core::GUID;

use crate::io::dir::DirHandle;
use crate::io::open::OpenOptions;

/// `CR_SUCCESS`.
const CR_SUCCESS: u32 = 0;
/// `ERROR_SUCCESS`: the answer that lets a removal go on.
const ERROR_SUCCESS: u32 = 0;
/// `PBT_APMSUSPEND`.
const PBT_APMSUSPEND: u32 = 4;
/// `PBT_APMRESUMESUSPEND`.
const PBT_APMRESUMESUSPEND: u32 = 7;
/// `PBT_APMRESUMEAUTOMATIC`.
const PBT_APMRESUMEAUTOMATIC: u32 = 18;

/// What happened to a volume the watch lives on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// The device is about to be removed.
    QueryRemove,
    /// The removal was refused or cancelled.
    QueryRemoveFailed,
    /// The device is being removed.
    RemovePending,
    /// The device was removed.
    RemoveComplete,
    /// The volume is being locked.
    Lock,
    /// The lock failed.
    LockFailed,
    /// The volume was unlocked.
    Unlock,
    /// The volume is being dismounted.
    Dismount,
    /// The dismount failed.
    DismountFailed,
    /// The volume was mounted.
    Mount,
    /// The `BitLocker` status changed.
    FveChange,
}

/// The handler of a [`HandleWatch`]: it must return at once.
pub type NoticeHandler = dyn Fn(Notice) + Send + Sync;

/// The notice of a custom event GUID, if it is one this module knows.
#[must_use]
pub fn notice_of_event(guid: &GUID) -> Option<Notice> {
    let table: [(&GUID, Notice); 7] = [
        (&GUID_IO_VOLUME_LOCK, Notice::Lock),
        (&GUID_IO_VOLUME_LOCK_FAILED, Notice::LockFailed),
        (&GUID_IO_VOLUME_UNLOCK, Notice::Unlock),
        (&GUID_IO_VOLUME_DISMOUNT, Notice::Dismount),
        (&GUID_IO_VOLUME_DISMOUNT_FAILED, Notice::DismountFailed),
        (&GUID_IO_VOLUME_MOUNT, Notice::Mount),
        (&GUID_IO_VOLUME_FVE_STATUS_CHANGE, Notice::FveChange),
    ];
    let same = |a: &GUID, b: &GUID| {
        a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
    };
    table
        .iter()
        .find(|(known, _)| same(known, guid))
        .map(|(_, notice)| *notice)
}

/// The notice for a plain action.
#[must_use]
pub const fn notice_of_action(action: CM_NOTIFY_ACTION) -> Option<Notice> {
    match action {
        CM_NOTIFY_ACTION_DEVICEQUERYREMOVE => Some(Notice::QueryRemove),
        CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED => Some(Notice::QueryRemoveFailed),
        CM_NOTIFY_ACTION_DEVICEREMOVEPENDING => Some(Notice::RemovePending),
        CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE => Some(Notice::RemoveComplete),
        _ => None,
    }
}

/// The context handed to a callback: freed after the registration is gone.
struct Context<T: ?Sized>(Arc<T>);

fn context_ptr<T: ?Sized>(handler: Arc<T>) -> *mut Context<T> {
    Box::into_raw(Box::new(Context(handler)))
}

/// A registration for a directory's volume.
#[derive(Debug)]
pub struct HandleWatch {
    notify: HCMNOTIFICATION,
    context: *mut Context<NoticeHandler>,
    /// The handle the registration watches; closed after the unregister.
    _dir: DirHandle,
}

// SAFETY: the notification handle and the context pointer are only used by
// `Drop`, which may run on any thread; the callback only reads the context.
unsafe impl Send for HandleWatch {}
// SAFETY: as above; nothing is mutated through a shared reference.
unsafe impl Sync for HandleWatch {}

unsafe extern "system" fn handle_callback(
    _notify: HCMNOTIFICATION,
    context: *const core::ffi::c_void,
    action: CM_NOTIFY_ACTION,
    data: *const CM_NOTIFY_EVENT_DATA,
    _size: u32,
) -> u32 {
    // SAFETY: `context` is the pointer `register` passed, alive until the
    // unregister returns, which waits for running callbacks.
    let context = unsafe { &*context.cast::<Context<NoticeHandler>>() };
    let notice = if action == CM_NOTIFY_ACTION_DEVICECUSTOMEVENT && !data.is_null() {
        // SAFETY: the system passes valid event data for the callback.
        let data = unsafe { &*data };
        // SAFETY: for a custom event on a handle filter the `DeviceHandle`
        // member of the union is the valid one.
        let guid = unsafe { data.u.DeviceHandle.EventGuid };
        notice_of_event(&guid)
    } else {
        notice_of_action(action)
    };
    if let Some(notice) = notice {
        (context.0)(notice);
    }
    ERROR_SUCCESS
}

impl HandleWatch {
    /// Registers `handler` for the volume of the directory at `path` (a `\\?\`
    /// path).
    ///
    /// # Errors
    ///
    /// The OS error when the directory cannot be opened or the registration
    /// fails (`CONFIGRET` as an `Other` error).
    pub fn register(path: &[u16], handler: Arc<NoticeHandler>) -> io::Result<Self> {
        let dir = DirHandle::open(path, OpenOptions::default())?;
        let context = context_ptr(handler);
        let filter = CM_NOTIFY_FILTER {
            cbSize: u32::try_from(size_of::<CM_NOTIFY_FILTER>()).unwrap_or(0),
            Flags: 0,
            FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE,
            Reserved: 0,
            u: windows_sys::Win32::Devices::DeviceAndDriverInstallation::CM_NOTIFY_FILTER_0 {
                DeviceHandle: CM_NOTIFY_FILTER_0_1 {
                    hTarget: dir.as_raw_handle(),
                },
            },
        };
        let mut notify: HCMNOTIFICATION = std::ptr::null_mut();
        // SAFETY: the filter is fully initialised and outlives the call; the
        // context stays alive until the unregister; `notify` is a valid out
        // pointer.
        let result = unsafe {
            CM_Register_Notification(
                &raw const filter,
                context.cast(),
                Some(handle_callback),
                &raw mut notify,
            )
        };
        if result != CR_SUCCESS {
            // SAFETY: the registration failed, so nothing else holds the
            // context.
            drop(unsafe { Box::from_raw(context) });
            return Err(io::Error::other(format!(
                "CM_Register_Notification failed with CONFIGRET {result}"
            )));
        }
        Ok(Self {
            notify,
            context,
            _dir: dir,
        })
    }
}

impl Drop for HandleWatch {
    fn drop(&mut self) {
        // SAFETY: `notify` came from a successful registration; the call
        // waits for callbacks that are running.
        unsafe { CM_Unregister_Notification(self.notify) };
        // SAFETY: the registration is gone, so no callback can use the
        // context any more.
        drop(unsafe { Box::from_raw(self.context) });
    }
}

/// The handler of an [`InterfaceWatch`]: `true` for an arrival.
pub type InterfaceHandler = dyn Fn(bool) + Send + Sync;

/// Arrival and removal of volume device interfaces.
#[derive(Debug)]
pub struct InterfaceWatch {
    notify: HCMNOTIFICATION,
    context: *mut Context<InterfaceHandler>,
}

// SAFETY: see `HandleWatch`.
unsafe impl Send for InterfaceWatch {}
// SAFETY: see `HandleWatch`.
unsafe impl Sync for InterfaceWatch {}

unsafe extern "system" fn interface_callback(
    _notify: HCMNOTIFICATION,
    context: *const core::ffi::c_void,
    action: CM_NOTIFY_ACTION,
    _data: *const CM_NOTIFY_EVENT_DATA,
    _size: u32,
) -> u32 {
    // SAFETY: as in `handle_callback`.
    let context = unsafe { &*context.cast::<Context<InterfaceHandler>>() };
    if action == CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL {
        (context.0)(true);
    } else if action == CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL {
        (context.0)(false);
    }
    ERROR_SUCCESS
}

impl InterfaceWatch {
    /// Registers `handler` for volume interfaces.
    ///
    /// # Errors
    ///
    /// The `CONFIGRET` of a failed registration.
    pub fn register(handler: Arc<InterfaceHandler>) -> io::Result<Self> {
        let context = context_ptr(handler);
        let filter = CM_NOTIFY_FILTER {
            cbSize: u32::try_from(size_of::<CM_NOTIFY_FILTER>()).unwrap_or(0),
            Flags: 0,
            FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
            Reserved: 0,
            u: windows_sys::Win32::Devices::DeviceAndDriverInstallation::CM_NOTIFY_FILTER_0 {
                DeviceInterface: CM_NOTIFY_FILTER_0_0 {
                    ClassGuid: GUID_DEVINTERFACE_VOLUME,
                },
            },
        };
        let mut notify: HCMNOTIFICATION = std::ptr::null_mut();
        // SAFETY: as in `HandleWatch::register`.
        let result = unsafe {
            CM_Register_Notification(
                &raw const filter,
                context.cast(),
                Some(interface_callback),
                &raw mut notify,
            )
        };
        if result != CR_SUCCESS {
            // SAFETY: the registration failed, so nothing else holds the
            // context.
            drop(unsafe { Box::from_raw(context) });
            return Err(io::Error::other(format!(
                "CM_Register_Notification failed with CONFIGRET {result}"
            )));
        }
        Ok(Self { notify, context })
    }
}

impl Drop for InterfaceWatch {
    fn drop(&mut self) {
        // SAFETY: as in `HandleWatch::drop`.
        unsafe { CM_Unregister_Notification(self.notify) };
        // SAFETY: as in `HandleWatch::drop`.
        drop(unsafe { Box::from_raw(self.context) });
    }
}

/// A power transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    /// The system is about to sleep; flush within about two seconds.
    Suspend,
    /// The system woke up.
    Resume,
}

/// The handler of a [`PowerWatch`]: it must return at once.
pub type PowerHandler = dyn Fn(PowerEvent) + Send + Sync;

/// Suspend and resume notifications.
#[derive(Debug)]
pub struct PowerWatch {
    registration: *mut core::ffi::c_void,
    context: *mut Context<PowerHandler>,
}

// SAFETY: see `HandleWatch`.
unsafe impl Send for PowerWatch {}
// SAFETY: see `HandleWatch`.
unsafe impl Sync for PowerWatch {}

unsafe extern "system" fn power_callback(
    context: *const core::ffi::c_void,
    kind: u32,
    _setting: *const core::ffi::c_void,
) -> u32 {
    // SAFETY: as in `handle_callback`.
    let context = unsafe { &*context.cast::<Context<PowerHandler>>() };
    match kind {
        PBT_APMSUSPEND => (context.0)(PowerEvent::Suspend),
        PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => (context.0)(PowerEvent::Resume),
        _ => {}
    }
    ERROR_SUCCESS
}

impl PowerWatch {
    /// Registers `handler`.
    ///
    /// # Errors
    ///
    /// The error code of a failed registration.
    pub fn register(handler: Arc<PowerHandler>) -> io::Result<Self> {
        let context = context_ptr(handler);
        let parameters = DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(power_callback),
            Context: context.cast(),
        };
        let mut registration: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: in callback mode `recipient` points at the parameters, which
        // are only read during the call; the context stays alive until the
        // unregister; `registration` is a valid out pointer.
        let result = unsafe {
            PowerRegisterSuspendResumeNotification(
                DEVICE_NOTIFY_CALLBACK,
                (&raw const parameters).cast_mut().cast(),
                &raw mut registration,
            )
        };
        if result != 0 {
            // SAFETY: the registration failed, so nothing else holds the
            // context.
            drop(unsafe { Box::from_raw(context) });
            return Err(io::Error::from_raw_os_error(result.cast_signed()));
        }
        Ok(Self {
            registration,
            context,
        })
    }
}

impl Drop for PowerWatch {
    fn drop(&mut self) {
        // SAFETY: the registration came from a successful call; the call waits
        // for a running callback.
        unsafe { PowerUnregisterSuspendResumeNotification(self.registration as isize) };
        // SAFETY: the registration is gone, so no callback can use the
        // context any more.
        drop(unsafe { Box::from_raw(self.context) });
    }
}
