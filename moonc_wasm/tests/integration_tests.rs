//! Integration tests for wasmoo_extern functionality
//! 
//! Tests the complete integration of cross-platform operations with the v8 JavaScript engine

use std::fs::{self, File};
use tempfile::TempDir;
use rstest::rstest;

use moonc_wasm::cross_platform::{CrossPlatformMetadataExt, PermissionsBuilder, RawFdExt, host_isatty};

#[cfg(test)]
mod file_operations_integration {
    use super::*;

    /// Test file descriptor operations across platforms
    #[rstest]
    fn test_file_descriptor_operations() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let file_path = temp_dir.path().join("fd_test.txt");
        
        // Create and open file
        let content = "File descriptor test content";
        fs::write(&file_path, content).expect("Failed to write file");
        
        let file = File::open(&file_path).expect("Failed to open file");
        
        // Test raw file descriptor extraction
        let raw_fd = file.as_raw_fd();
        
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                assert!(raw_fd >= 0, "Unix file descriptor should be non-negative");
                
                // Test host_isatty function
                let tty_result = host_isatty(raw_fd);
                assert_eq!(tty_result, 0, "Regular file should not be a TTY");
            } else if #[cfg(windows)] {
                // On Windows, raw_fd is a HANDLE
                println!("Windows file handle: {:?}", raw_fd);
                
                // Test host_isatty function (should return 0 for regular files)
                let tty_result = host_isatty(raw_fd);
                assert_eq!(tty_result, 0, "Regular file should not be a TTY on Windows");
            }
        }
        
        println!("✅ File descriptor operations work across platforms");
    }

    /// Test standard I/O file descriptors
    #[rstest]
    #[case::stdin(0, "stdin")]
    #[case::stdout(1, "stdout")]
    #[case::stderr(2, "stderr")]
    fn test_standard_io_descriptors(#[case] fd: i32, #[case] name: &str) {
        // Test host_isatty on standard file descriptors
        let tty_result = host_isatty(fd as _);
        
        // Standard streams might or might not be TTYs depending on environment
        assert!(tty_result == 0 || tty_result == 1, 
               "{} host_isatty should return 0 or 1", name);
        
        println!("✅ {} (fd={}) host_isatty result: {}", name, fd, tty_result);
    }
}

#[cfg(test)]
mod metadata_integration_tests {
    use super::*;

    /// Test complete metadata workflow as used in wasmoo_extern
    #[rstest]
    #[case::regular_file("regular.txt", "Regular file content")]
    #[case::executable_script("script.sh", "#!/bin/bash\necho 'Hello World'")]
    fn test_complete_metadata_workflow(
        #[case] filename: &str,
        #[case] content: &str,
    ) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let file_path = temp_dir.path().join(filename);
        
        // Create file with specific content
        fs::write(&file_path, content).expect("Failed to write file");
        
        // Set executable permissions for script files
        if filename.ends_with(".sh") {
            let permissions = PermissionsBuilder::from_mode(0o755);
            fs::set_permissions(&file_path, permissions).expect("Failed to set permissions");
        }
        
        // Get metadata and extract all fields (as done in wasmoo_extern stat functions)
        let metadata = fs::metadata(&file_path).expect("Failed to get metadata");
        
        // Extract all metadata fields using cross-platform methods
        let kind = if metadata.is_file() { 0 } else { 1 }; // File type classification
        let dev = metadata.cross_dev();
        let ino = metadata.cross_ino();
        let mode = metadata.cross_mode();
        let nlink = metadata.cross_nlink();
        let uid = metadata.cross_uid();
        let gid = metadata.cross_gid();
        let rdev = metadata.cross_rdev();
        let size = metadata.cross_size();
        let atime = metadata.cross_atime();
        let mtime = metadata.cross_mtime();
        let ctime = metadata.cross_ctime();
        
        // Verify all fields have reasonable values
        assert_eq!(kind, 0, "Should be classified as regular file");
        assert_eq!(size as usize, content.len(), "Size should match content");
        assert!(nlink >= 1, "Should have at least one link");
        assert!(mtime > 0, "Modification time should be positive");
        
        // Simulate v8 object creation (as done in wasmoo_extern)
        let stat_object = format!(
            "{{ kind: {}, dev: {}, ino: {}, mode: {}, nlink: {}, uid: {}, gid: {}, rdev: {}, size: {}, atime: {}, mtime: {}, ctime: {} }}",
            kind, dev, ino, mode, nlink, uid, gid, rdev, size, atime, mtime, ctime
        );
        
        println!("✅ Complete metadata workflow for {} - {}", filename, stat_object);
    }

    /// Test metadata operations on different file types
    #[rstest]
    fn test_directory_metadata() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let dir_path = temp_dir.path().join("test_subdir");
        
        // Create subdirectory
        fs::create_dir(&dir_path).expect("Failed to create directory");
        
        // Get directory metadata
        let metadata = fs::metadata(&dir_path).expect("Failed to get directory metadata");
        
        // Test directory-specific metadata
        assert!(metadata.is_dir(), "Should be identified as directory");
        
        let kind = if metadata.is_dir() { 1 } else { 0 };
        let mode = metadata.cross_mode();
        let nlink = metadata.cross_nlink();
        
        assert_eq!(kind, 1, "Directory should have kind = 1");
        assert!(mode > 0, "Directory should have valid mode");
        
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                // On Unix, directories typically have nlink >= 2 (. and ..)
                assert!(nlink >= 2, "Unix directory should have nlink >= 2");
            } else if #[cfg(windows)] {
                // On Windows, nlink behavior may differ
                assert!(nlink >= 1, "Windows directory should have nlink >= 1");
            }
        }
        
        println!("✅ Directory metadata - Kind: {}, Mode: 0o{:o}, Links: {}", kind, mode, nlink);
    }
}

#[cfg(test)]
mod error_handling_integration {
    use super::*;

    /// Test error handling in cross-platform operations
    #[rstest]
    fn test_nonexistent_file_handling() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let nonexistent_path = temp_dir.path().join("does_not_exist.txt");
        
        // Attempt to get metadata for nonexistent file
        let result = fs::metadata(&nonexistent_path);
        assert!(result.is_err(), "Should fail for nonexistent file");
        
        // Verify error type
        let error = result.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "Should be NotFound error");
        
        println!("✅ Nonexistent file error handling works correctly");
    }

    /// Test permission denied scenarios
    #[rstest]
    fn test_permission_scenarios() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let file_path = temp_dir.path().join("restricted.txt");
        
        // Create file
        fs::write(&file_path, "restricted content").expect("Failed to create file");
        
        // Try to set very restrictive permissions
        let permissions = PermissionsBuilder::from_mode(0o000);
        let perm_result = fs::set_permissions(&file_path, permissions);
        
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                // On Unix, this should succeed
                assert!(perm_result.is_ok(), "Setting restrictive permissions should succeed on Unix");
                
                // Verify the file is now readonly
                let metadata = fs::metadata(&file_path).expect("Failed to get metadata");
                assert!(metadata.permissions().readonly(), "File should be readonly");
            } else if #[cfg(windows)] {
                // On Windows, behavior may vary
                println!("Windows restrictive permissions result: {:?}", perm_result);
            }
        }
        
        println!("✅ Permission scenarios handled appropriately");
    }
}

#[cfg(test)]
mod performance_integration {
    use super::*;

    /// Test performance of metadata operations under load
    #[rstest]
    fn test_metadata_performance_integration() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        // Create multiple test files
        let file_count = 100;
        let mut file_paths = Vec::new();
        
        for i in 0..file_count {
            let file_path = temp_dir.path().join(format!("perf_test_{}.txt", i));
            let content = format!("Performance test file {} content", i);
            fs::write(&file_path, &content).expect("Failed to create test file");
            file_paths.push(file_path);
        }
        
        // Measure time for complete metadata extraction workflow
        let start = std::time::Instant::now();
        
        for file_path in &file_paths {
            let metadata = fs::metadata(file_path).expect("Failed to get metadata");
            
            // Extract all metadata fields (simulating wasmoo_extern usage)
            let _dev = metadata.cross_dev();
            let _ino = metadata.cross_ino();
            let _mode = metadata.cross_mode();
            let _nlink = metadata.cross_nlink();
            let _uid = metadata.cross_uid();
            let _gid = metadata.cross_gid();
            let _rdev = metadata.cross_rdev();
            let _size = metadata.cross_size();
            let _atime = metadata.cross_atime();
            let _mtime = metadata.cross_mtime();
            let _ctime = metadata.cross_ctime();
        }
        
        let duration = start.elapsed();
        let avg_time = duration.as_micros() / file_count as u128;
        
        // Performance should be reasonable (less than 1ms per file)
        assert!(avg_time < 1000, "Average metadata extraction should be under 1ms per file");
        
        println!("✅ Performance integration - {} files in {:?} (avg: {}μs per file)", 
                 file_count, duration, avg_time);
    }

    /// Test concurrent metadata operations
    #[rstest]
    fn test_concurrent_metadata_operations() {
        use std::thread;
        use std::sync::Arc;
        
        let temp_dir = Arc::new(TempDir::new().expect("Failed to create temp dir"));
        let file_path = temp_dir.path().join("concurrent_test.txt");
        
        // Create test file
        fs::write(&file_path, "Concurrent access test").expect("Failed to create file");
        
        // Spawn multiple threads to access metadata concurrently
        let handles: Vec<_> = (0..10).map(|i| {
            let file_path = file_path.clone();
            thread::spawn(move || {
                for _ in 0..10 {
                    let metadata = fs::metadata(&file_path).expect("Failed to get metadata");
                    let _size = metadata.cross_size();
                    let _mode = metadata.cross_mode();
                    let _mtime = metadata.cross_mtime();
                }
                i
            })
        }).collect();
        
        // Wait for all threads to complete
        for handle in handles {
            handle.join().expect("Thread should complete successfully");
        }
        
        println!("✅ Concurrent metadata operations completed successfully");
    }
}
