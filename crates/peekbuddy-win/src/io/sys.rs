//! The windows-sys items this module tree uses, in one place, so that a
//! move to a later windows-sys is a mechanical change (#83).

pub use windows_sys::Win32::Foundation::{
    ERROR_NOT_FOUND, INVALID_HANDLE_VALUE, RtlNtStatusToDosError, WAIT_TIMEOUT,
};
pub use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW, GetVolumeInformationByHandleW,
    GetVolumeNameForVolumeMountPointW, GetVolumePathNameW, OPEN_EXISTING, VOLUME_NAME_DOS,
};
pub use windows_sys::Win32::System::IO::{
    CancelIoEx, CreateIoCompletionPort, GetQueuedCompletionStatusEx, OVERLAPPED, OVERLAPPED_ENTRY,
    PostQueuedCompletionStatus,
};
