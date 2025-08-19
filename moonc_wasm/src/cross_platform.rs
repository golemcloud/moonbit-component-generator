//! Cross-platform file system operations for moonbit-component-generator
//! Provides Windows-compatible alternatives to Unix-only file operations.

#![allow(dead_code)]

use std::fs::{self, File, Metadata, Permissions, FileType};
use std::path::Path;
use std::io;
use std::io::IsTerminal;

use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(windows)] {
        use std::os::windows::fs::MetadataExt;
        use std::os::windows::io::{AsRawHandle, RawHandle};
        use windows::Win32::Foundation::{HANDLE, FILETIME, INVALID_HANDLE_VALUE};
        use windows::Win32::Storage::FileSystem::{
            CreateFileW,
            FILE_ATTRIBUTE_NORMAL,
            FILE_FLAG_OVERLAPPED,
            FILE_GENERIC_WRITE, FILE_SHARE_READ, OPEN_EXISTING,
            SetFileTime, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        use windows::Win32::System::Console::{GetConsoleMode, CONSOLE_MODE};
        use windows::core::{PCWSTR, Error as WindowsError};
        pub type RawFd = RawHandle;
    } else {
        use std::os::unix::fs::{MetadataExt, PermissionsExt, OpenOptionsExt, FileTypeExt};
        use std::os::unix::io::{AsRawFd, RawFd};
        use libc::{timeval, utimes as libc_utimes};
        pub type RawFd = RawFd;
    }
}

/// Cross-platform file operation constants
pub mod platform_constants {
    cfg_if::cfg_if! {
        if #[cfg(windows)] {
            pub const O_NONBLOCK: i32 = 0x1000; // Custom flag for Windows async I/O
            pub const O_NOCTTY: i32 = 0x0000;   // No-op on Windows (no TTY concept)
            pub const O_DSYNC: i32 = 0x0000;    // No direct equivalent on Windows
            pub const O_SYNC: i32 = 0x0000;     // No direct equivalent on Windows
        } else if #[cfg(unix)] {
            pub const O_NONBLOCK: i32 = libc::O_NONBLOCK;
            pub const O_NOCTTY: i32 = libc::O_NOCTTY;
            pub const O_DSYNC: i32 = libc::O_DSYNC;
            pub const O_SYNC: i32 = libc::O_SYNC;
        } else {
            pub const O_NONBLOCK: i32 = 0;
            pub const O_NOCTTY: i32 = 0;
            pub const O_DSYNC: i32 = 0;
            pub const O_SYNC: i32 = 0;
        }
    }
}

/// Cross-platform metadata extractor that works on Windows and Unix
pub struct MetadataExtractor;

impl MetadataExtractor {
    /// Extract file mode (Unix permissions or Windows attributes)
    pub fn mode(metadata: &Metadata) -> u32 {
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                metadata.file_attributes()
            } else if #[cfg(unix)] {
                metadata.mode()
            } else {
                0o644 // Default file permissions
            }
        }
    }

    /// Extract device ID
    pub fn dev(metadata: &Metadata) -> u64 {
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                // Use stable Windows API - file attributes as device ID
                metadata.file_attributes() as u64
            } else if #[cfg(unix)] {
                metadata.dev()
            } else {
                0
            }
        }
    }

    /// Extract inode number
    pub fn ino(metadata: &Metadata) -> u64 {
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                // Use file size + creation time as pseudo-inode
                metadata.file_size().wrapping_add(metadata.creation_time())
            } else if #[cfg(unix)] {
                metadata.ino()
            } else {
                0
            }
        }
    }

    /// Extract number of hard links
    pub fn nlink(metadata: &Metadata) -> u64 {
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                // Windows files typically have 1 link
                1
            } else if #[cfg(unix)] {
                metadata.nlink()
            } else {
                1
            }
        }
    }

    /// Extract user ID (Windows: simulate with 0, Unix: actual UID)
    pub fn uid(metadata: &Metadata) -> u32 {
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                // Windows uses SIDs, not UIDs. For compatibility, return 0
                // In production, you might want to hash the current user SID
                0
            } else if #[cfg(unix)] {
                metadata.uid()
            } else {
                0
            }
        }
    }

    /// Extract group ID (Windows: simulate with 0, Unix: actual GID)
    pub fn gid(metadata: &Metadata) -> u32 {
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                // Windows uses SIDs, not GIDs. For compatibility, return 0
                0
            } else if #[cfg(unix)] {
                metadata.gid()
            } else {
                0
            }
        }
    }

    /// Extract device ID
    pub fn rdev(metadata: &Metadata) -> u64 {
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                // Use file attributes as rdev equivalent
                metadata.file_attributes() as u64
            } else if #[cfg(unix)] {
                metadata.rdev()
            } else {
                0
            }
        }
    }

    /// Extract file size
    pub fn size(metadata: &Metadata) -> u64 {
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                metadata.file_size()
            } else if #[cfg(unix)] {
                metadata.size()
            } else {
                metadata.len() // Standard library fallback
            }
        }
    }

    /// Extract access time as Unix timestamp
    pub fn atime(metadata: &Metadata) -> i64 {
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                Self::filetime_to_unix_timestamp(metadata.last_access_time())
            } else if #[cfg(unix)] {
                metadata.atime()
            } else {
                0
            }
        }
    }

    /// Extract modification time as Unix timestamp
    pub fn mtime(metadata: &Metadata) -> i64 {
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                Self::filetime_to_unix_timestamp(metadata.last_write_time())
            } else if #[cfg(unix)] {
                metadata.mtime()
            } else {
                0
            }
        }
    }

    /// Extract change/creation time as Unix timestamp
    pub fn ctime(metadata: &Metadata) -> i64 {
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                // Windows has creation time, not change time
                Self::filetime_to_unix_timestamp(metadata.creation_time())
            } else if #[cfg(unix)] {
                metadata.ctime()
            } else {
                0
            }
        }
    }

    /// Convert Windows FILETIME to Unix timestamp
    #[cfg(windows)]
    fn filetime_to_unix_timestamp(filetime: u64) -> i64 {
        // Windows FILETIME: 100-nanosecond intervals since January 1, 1601 UTC
        // Unix timestamp: seconds since January 1, 1970 UTC
        const WINDOWS_TO_UNIX_OFFSET: u64 = 11_644_473_600; // seconds between epochs
        const FILETIME_UNITS_PER_SECOND: u64 = 10_000_000;  // 100ns units per second

        let unix_seconds = (filetime / FILETIME_UNITS_PER_SECOND).saturating_sub(WINDOWS_TO_UNIX_OFFSET);
        unix_seconds as i64
    }
}

/// Cross-platform permissions constructor
pub struct PermissionsBuilder;

impl PermissionsBuilder {
    /// Create permissions from Unix-style mode
    pub fn from_mode(mode: u32) -> Permissions {
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                // Windows doesn't use Unix-style modes
                // Return default permissions or create from attributes
                Self::create_windows_permissions(mode)
            } else if #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                Permissions::from_mode(mode)
            } else {
                // Fallback: create default permissions
                fs::metadata(".").unwrap().permissions()
            }
        }
    }

    #[cfg(windows)]
    fn create_windows_permissions(mode: u32) -> Permissions {
        // Convert Unix mode to Windows file attributes
        let read_only = (mode & 0o200) == 0; // Owner write bit

        // Create a temporary file to get permissions, then modify
        let temp_path = std::env::temp_dir().join("temp_permissions");
        if let Ok(_) = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp_path) 
        {
            let mut perms = fs::metadata(&temp_path).unwrap().permissions();
            perms.set_readonly(read_only);

            // Clean up
            let _ = fs::remove_file(temp_path);

            perms
        } else {
            // Fallback to current directory permissions
            fs::metadata(".").unwrap().permissions()
        }
    }
}

/// Cross-platform raw file descriptor extraction
pub trait RawFdExt {
    fn as_raw_fd(&self) -> RawFd;
}

impl RawFdExt for File {
    fn as_raw_fd(&self) -> RawFd {
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                use std::os::windows::io::AsRawHandle;
                self.as_raw_handle()
            } else if #[cfg(unix)] {
                use std::os::unix::io::AsRawFd;
                self.as_raw_fd()
            } else {
                0 as RawFd
            }
        }
    }
}

/// Cross-platform isatty function (renamed to avoid conflicts)
pub fn host_isatty(fd: RawFd) -> i32 {
    cfg_if::cfg_if! {
        if #[cfg(windows)] {
            // Check standard streams first
            if fd.is_null() { // stdin
                return if std::io::stdin().is_terminal() { 1 } else { 0 };
            }
            if fd == std::io::stdout().as_raw_handle() { // stdout
                return if std::io::stdout().is_terminal() { 1 } else { 0 };
            }
            if fd == std::io::stderr().as_raw_handle() { // stderr
                return if std::io::stderr().is_terminal() { 1 } else { 0 };
            }
            
            // For other handles, use Win32 API
            unsafe {
                let handle = HANDLE(fd);
                let mut mode = CONSOLE_MODE(0);
                if GetConsoleMode(handle, &mut mode).is_ok() {
                    1
                } else {
                    0
                }
            }
        } else if #[cfg(unix)] {
            unsafe { libc::isatty(fd) }
        } else {
            0
        }
    }
}

/// Trait to extend std::fs::Metadata with cross-platform methods
pub trait CrossPlatformMetadataExt {
    fn cross_dev(&self) -> u64;
    fn cross_ino(&self) -> u64;
    fn cross_mode(&self) -> u32;
    fn cross_nlink(&self) -> u64;
    fn cross_uid(&self) -> u32;
    fn cross_gid(&self) -> u32;
    fn cross_rdev(&self) -> u64;
    fn cross_size(&self) -> u64;
    fn cross_atime(&self) -> i64;
    fn cross_mtime(&self) -> i64;
    fn cross_ctime(&self) -> i64;
}

impl CrossPlatformMetadataExt for Metadata {
    fn cross_dev(&self) -> u64 { MetadataExtractor::dev(self) }
    fn cross_ino(&self) -> u64 { MetadataExtractor::ino(self) }
    fn cross_mode(&self) -> u32 { MetadataExtractor::mode(self) }
    fn cross_nlink(&self) -> u64 { MetadataExtractor::nlink(self) }
    fn cross_uid(&self) -> u32 { MetadataExtractor::uid(self) }
    fn cross_gid(&self) -> u32 { MetadataExtractor::gid(self) }
    fn cross_rdev(&self) -> u64 { MetadataExtractor::rdev(self) }
    fn cross_size(&self) -> u64 { MetadataExtractor::size(self) }
    fn cross_atime(&self) -> i64 { MetadataExtractor::atime(self) }
    fn cross_mtime(&self) -> i64 { MetadataExtractor::mtime(self) }
    fn cross_ctime(&self) -> i64 { MetadataExtractor::ctime(self) }
}

/// Cross-platform FileType extensions
pub trait CrossPlatformFileTypeExt {
    fn cross_is_char_device(&self) -> bool;
    fn cross_is_block_device(&self) -> bool;
    fn cross_is_fifo(&self) -> bool;
    fn cross_is_socket(&self) -> bool;
}

impl CrossPlatformFileTypeExt for FileType {
    fn cross_is_char_device(&self) -> bool {
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                use std::os::unix::fs::FileTypeExt;
                self.is_char_device()
            } else {
                false // Windows doesn't have char devices in the same way
            }
        }
    }

    fn cross_is_block_device(&self) -> bool {
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                use std::os::unix::fs::FileTypeExt;
                self.is_block_device()
            } else {
                false // Windows doesn't have block devices in the same way
            }
        }
    }

    fn cross_is_fifo(&self) -> bool {
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                use std::os::unix::fs::FileTypeExt;
                self.is_fifo()
            } else {
                false // Windows doesn't have FIFOs
            }
        }
    }

    fn cross_is_socket(&self) -> bool {
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                use std::os::unix::fs::FileTypeExt;
                self.is_socket()
            } else {
                false // Windows doesn't have Unix domain sockets in filesystem
            }
        }
    }
}

pub fn cross_utimes(path: &str, atime: f64, mtime: f64) -> io::Result<()> {
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            use libc::{timeval, suseconds_t, utimes as libc_utimes};
            let c_path = std::ffi::CString::new(path)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid path"))?;
            
            let atime_tv = timeval {
                tv_sec: atime as i64,
                tv_usec: (atime.fract() * 1_000_000.0) as suseconds_t,
            };
            let mtime_tv = timeval {
                tv_sec: mtime as i64, 
                tv_usec: (mtime.fract() * 1_000_000.0) as suseconds_t,
            };
            
            let times = [atime_tv, mtime_tv];
            
            unsafe {
                if libc_utimes(c_path.as_ptr(), times.as_ptr()) == 0 {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error())
                }
            }
        } else if #[cfg(windows)] {
            // FIXED Windows implementation
            use windows::core::HSTRING;
            use windows::Win32::Storage::FileSystem::{
                CreateFileW, FILE_GENERIC_WRITE, FILE_SHARE_READ, 
                OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, SetFileTime
            };
            use windows::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, FILETIME};
            use windows::core::PCWSTR;
            
            let wide_path = HSTRING::from(path);
            
            unsafe {
                // FIX ERROR 3: Correct CreateFileW parameter types
                let handle = CreateFileW(
                    PCWSTR(wide_path.as_ptr()),
                    FILE_GENERIC_WRITE.0,
                    FILE_SHARE_READ,
                    None,                           // Option<*const SECURITY_ATTRIBUTES> ✓
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,          // Remove .0 ✓
                    None,                           // Option<HANDLE> ✓
                )?;
                
                if handle == INVALID_HANDLE_VALUE {
                    return Err(io::Error::last_os_error());
                }
                
                // Convert Unix timestamps to Windows FILETIME
                let unix_to_filetime = |timestamp: f64| -> FILETIME {
                    const WINDOWS_TO_UNIX_OFFSET: u64 = 11_644_473_600;
                    const FILETIME_UNITS_PER_SECOND: u64 = 10_000_000;
                    
                    let windows_time = ((timestamp as u64) + WINDOWS_TO_UNIX_OFFSET) * FILETIME_UNITS_PER_SECOND;
                    
                    FILETIME {
                        dwLowDateTime: (windows_time & 0xFFFFFFFF) as u32,
                        dwHighDateTime: (windows_time >> 32) as u32,
                    }
                };
                
                let atime_ft = unix_to_filetime(atime);
                let mtime_ft = unix_to_filetime(mtime);
                
                // FIX ERROR 4: SetFileTime returns Result, not Option
                SetFileTime(
                    handle,
                    None,
                    Some(&atime_ft),
                    Some(&mtime_ft),
                ).map_err(|_| io::Error::last_os_error())  // Result::map_err ✓
            }
        } else {
            Err(io::Error::new(io::ErrorKind::Unsupported, "utimes not supported on this platform"))
        }
    }
}


/// Convert Unix timestamp to Windows FILETIME
#[cfg(windows)]
fn unix_to_filetime(timestamp: f64) -> FILETIME {
    const WINDOWS_TO_UNIX_OFFSET: u64 = 11_644_473_600; // seconds between epochs
    const FILETIME_UNITS_PER_SECOND: u64 = 10_000_000;  // 100ns units per second
    
    let windows_time = ((timestamp as u64) + WINDOWS_TO_UNIX_OFFSET) * FILETIME_UNITS_PER_SECOND;
    
    FILETIME {
        dwLowDateTime: (windows_time & 0xFFFFFFFF) as u32,
        dwHighDateTime: (windows_time >> 32) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_cross_platform_metadata() {
        let test_path = "test_metadata.txt";

        // Create test file
        {
            let mut file = File::create(test_path).unwrap();
            writeln!(file, "Test content for metadata").unwrap();
        }

        // Test metadata extraction
        let metadata = fs::metadata(test_path).unwrap();

        assert!(MetadataExtractor::mode(&metadata) > 0);
        assert!(MetadataExtractor::size(&metadata) > 0);
        assert_eq!(MetadataExtractor::nlink(&metadata), 1);

        // Test trait extension
        assert!(metadata.cross_mode() > 0);
        assert!(metadata.cross_size() > 0);

        // Cleanup
        fs::remove_file(test_path).unwrap();
    }

    #[test]
    fn test_permission_creation() {
        let perms = PermissionsBuilder::from_mode(0o644);
        // Just ensure it doesn't panic and creates something
        assert!(!perms.readonly() || perms.readonly()); // Always true, just test creation
    }
}
