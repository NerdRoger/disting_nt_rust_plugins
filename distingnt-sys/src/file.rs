//! SD filesystem access. All calls must occur within an NT_callFileSystemCode
//! callback. Handles and filesystem state must not be assumed to persist beyond
//! that callback; retain paths and reopen files on subsequent callbacks.
use core::ffi::{c_char, c_void};
pub type NtFileHandle = u32;
pub type NtFileResult = u8;
pub const FILE_OK: NtFileResult = 0;
pub const DISK_ERR: NtFileResult = 1;
pub const INT_ERR: NtFileResult = 2;
pub const NOT_READY: NtFileResult = 3;
pub const NO_FILE: NtFileResult = 4;
pub const NO_PATH: NtFileResult = 5;
pub const INVALID_NAME: NtFileResult = 6;
pub const DENIED: NtFileResult = 7;
pub const EXIST: NtFileResult = 8;
pub const INVALID_OBJECT: NtFileResult = 9;
pub const WRITE_PROTECTED: NtFileResult = 10;
pub const INVALID_DRIVE: NtFileResult = 11;
pub const NOT_ENABLED: NtFileResult = 12;
pub const NO_FILESYSTEM: NtFileResult = 13;
pub const TIMEOUT: NtFileResult = 14;
pub const LOCKED: NtFileResult = 15;
pub const NOT_ENOUGH_CORE: NtFileResult = 16;
pub const INVALID_PARAMETER: NtFileResult = 19;
pub const INVALID_FILE_HANDLE: NtFileResult = 100;
pub const MODE_READ: i32 = 0x01;
pub const MODE_WRITE: i32 = 0x02;
pub const MODE_OPEN_EXISTING: i32 = 0;
pub const MODE_CREATE_NEW: i32 = 0x04;
pub const MODE_CREATE_ALWAYS: i32 = 0x08;
pub const MODE_OPEN_ALWAYS: i32 = 0x10;
pub const MODE_OPEN_APPEND: i32 = 0x30;
pub const FIND_FILES: u8 = 1;
pub const FIND_FOLDERS: u8 = 2;
pub const FIND_ALL: u8 = 3;
pub const READ_ONLY: u8 = 1;
pub const HIDDEN: u8 = 2;
pub const SYSTEM: u8 = 4;
pub const DIRECTORY: u8 = 0x10;
pub const ARCHIVE: u8 = 0x20;
pub const NUM_AVAILABLE_FILE_HANDLES: usize = 8;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtFileInfo {
    pub size: u64,
    pub date: u16,
    pub time: u16,
    pub attrib: u8,
    pub name: [c_char; 256],
}
pub type FileSystemCallback = unsafe extern "C" fn(*mut c_void);
unsafe extern "C" {
    pub fn NT_callFileSystemCode(callback: Option<FileSystemCallback>, data: *mut c_void) -> bool;
    pub fn NT_fileOpen(handle: NtFileHandle, filename: *const c_char, mode: i32) -> NtFileResult;
    pub fn NT_fileClose(handle: NtFileHandle) -> NtFileResult;
    pub fn NT_fileRead(
        handle: NtFileHandle,
        dst: *mut c_void,
        bytes: u32,
        bytes_read: *mut u32,
    ) -> NtFileResult;
    pub fn NT_fileWrite(
        handle: NtFileHandle,
        src: *const c_void,
        bytes: u32,
        bytes_written: *mut u32,
    ) -> NtFileResult;
    pub fn NT_fileSeek(handle: NtFileHandle, pos: u32) -> NtFileResult;
    pub fn NT_ftell(handle: NtFileHandle) -> u32;
    pub fn NT_fileInfo(path: *const c_char, info: *mut NtFileInfo) -> NtFileResult;
    pub fn NT_fileDelete(path: *const c_char) -> NtFileResult;
    pub fn NT_chdir(path: *const c_char) -> NtFileResult;
    pub fn NT_mkdir(path: *const c_char) -> NtFileResult;
    pub fn NT_findFiles(pattern: *const c_char, flags: u8) -> u32;
    pub fn NT_getFoundFile(index: u32, buffer: *mut c_char, size: usize);
    pub fn NT_getFileScratchBuffer(size: *mut usize) -> *mut c_void;
}
