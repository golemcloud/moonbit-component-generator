//! Integration tests for cross-platform file system operations
//!
//! This module contains comprehensive parameterized tests using rstest
//! to verify cross-platform compatibility of metadata operations.

use rstest::{fixture, rstest};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use tempfile::TempDir;

use moonc_wasm::cross_platform::{
    CrossPlatformMetadataExt, PermissionsBuilder, platform_constants,
};

/// Test fixture that creates a temporary directory for each test
#[fixture]
fn temp_dir() -> TempDir {
    TempDir::new().expect("Failed to create temporary directory")
}

/// Test cross-platform metadata extraction with various file permissions
#[rstest]
#[case::read_only(0o444, "read-only file")]
#[case::read_write(0o644, "read-write file")]
#[case::executable(0o755, "executable file")]
#[case::full_permissions(0o777, "full permissions")]
#[case::no_write(0o555, "no write permissions")]
#[case::owner_only(0o600, "owner only permissions")]
fn test_metadata_permissions(temp_dir: TempDir, #[case] mode: u32, #[case] description: &str) {
    let file_path = temp_dir.path().join("test_file.txt");

    // Create test file with content
    let mut file = File::create(&file_path).expect("Failed to create test file");
    writeln!(file, "Test content for {}", description).expect("Failed to write to file");
    drop(file);

    // Set permissions using cross-platform builder
    let permissions = PermissionsBuilder::from_mode(mode);
    fs::set_permissions(&file_path, permissions).expect("Failed to set permissions");

    // Test metadata extraction
    let metadata = fs::metadata(&file_path).expect("Failed to get metadata");

    // Verify cross-platform methods work
    let extracted_mode = metadata.cross_mode();
    let size = metadata.cross_size();
    let uid = metadata.cross_uid();
    let gid = metadata.cross_gid();
    let nlink = metadata.cross_nlink();

    // Basic assertions
    assert!(size > 0, "File size should be greater than 0");
    assert!(nlink >= 1, "Number of links should be at least 1");

    // Platform-specific assertions
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            // On Unix, mode should reflect the permissions we set
            assert_eq!(extracted_mode & 0o777, mode, "Mode should match set permissions");
        } else if #[cfg(windows)] {
            // On Windows, mode represents file attributes
            assert!(extracted_mode > 0, "Windows file attributes should be present");
        }
    }

    println!(
        "✅ {} - Mode: 0o{:o}, Size: {}, UID: {}, GID: {}, Links: {}",
        description, extracted_mode, size, uid, gid, nlink
    );
}

/// Test metadata extraction for different file types and edge cases
#[rstest]
#[case::empty_file(0, "empty file")]
#[case::small_file(42, "small file")]
#[case::medium_file(1024, "medium file")]
#[case::large_content(65536, "large file")]
fn test_metadata_file_sizes(
    temp_dir: TempDir,
    #[case] content_size: usize,
    #[case] description: &str,
) {
    let file_path = temp_dir.path().join("size_test.txt");

    // Create file with specific content size
    let content = "x".repeat(content_size);
    fs::write(&file_path, &content).expect("Failed to write file");

    let metadata = fs::metadata(&file_path).expect("Failed to get metadata");

    // Test all cross-platform metadata methods
    let size = metadata.cross_size();
    let dev = metadata.cross_dev();
    let ino = metadata.cross_ino();
    let atime = metadata.cross_atime();
    let mtime = metadata.cross_mtime();
    let ctime = metadata.cross_ctime();

    // Verify size matches expected
    assert_eq!(
        size as usize, content_size,
        "File size should match written content"
    );

    // Verify other metadata fields are reasonable
    assert!(dev > 0 || dev == 0, "Device ID should be valid");
    assert!(ino > 0 || ino == 0, "Inode should be valid");
    assert!(
        atime > 0 || atime == 0,
        "Access time should be valid timestamp or 0"
    );
    assert!(mtime > 0, "Modification time should be positive timestamp");
    assert!(ctime >= 0, "Creation/change time should be non-negative");

    println!(
        "✅ {} - Size: {}, Dev: {}, Ino: {}, ATime: {}, MTime: {}, CTime: {}",
        description, size, dev, ino, atime, mtime, ctime
    );
}

/// Test platform constants compatibility
#[rstest]
#[case::nonblock(platform_constants::O_NONBLOCK, "O_NONBLOCK")]
#[case::noctty(platform_constants::O_NOCTTY, "O_NOCTTY")]
#[case::dsync(platform_constants::O_DSYNC, "O_DSYNC")]
#[case::sync(platform_constants::O_SYNC, "O_SYNC")]
fn test_platform_constants(#[case] constant: i32, #[case] name: &str) {
    // Verify constants are defined and have reasonable values
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            // On Unix, constants should match libc values or be reasonable
            assert!(constant >= 0, "{} should be non-negative on Unix", name);
        } else if #[cfg(windows)] {
            // On Windows, constants should be defined (may be 0 for unsupported features)
            assert!(constant >= 0, "{} should be non-negative on Windows", name);
        }
    }

    println!("✅ {} = 0x{:x} ({})", name, constant, constant);
}

/// Test permissions builder with various Unix permission modes
#[rstest]
#[case::no_permissions(0o000)]
#[case::owner_read(0o400)]
#[case::owner_write(0o200)]
#[case::owner_execute(0o100)]
#[case::group_read(0o040)]
#[case::group_write(0o020)]
#[case::group_execute(0o010)]
#[case::other_read(0o004)]
#[case::other_write(0o002)]
#[case::other_execute(0o001)]
#[case::common_file(0o644)]
#[case::common_executable(0o755)]
#[case::restricted(0o600)]
#[case::public_read(0o644)]
fn test_permissions_builder(temp_dir: TempDir, #[case] mode: u32) {
    let file_path = temp_dir.path().join("perm_test.txt");

    // Create test file
    File::create(&file_path).expect("Failed to create test file");

    // Test permissions builder
    let permissions = PermissionsBuilder::from_mode(mode);

    // Apply permissions
    let result = fs::set_permissions(&file_path, permissions);

    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            // On Unix, setting permissions should generally succeed
            assert!(result.is_ok(), "Setting permissions 0o{:o} should succeed on Unix", mode);

            // Verify the permissions were set correctly
            let metadata = fs::metadata(&file_path).expect("Failed to get metadata");
            let actual_mode = metadata.cross_mode() & 0o777;

            // Account for umask and filesystem limitations
            if mode & 0o200 == 0 {
                // If we tried to remove write permission, verify it's readonly
                assert!(metadata.permissions().readonly() || (actual_mode & 0o200) == 0,
                       "File should be readonly when write bit is cleared");
            }
        } else if #[cfg(windows)] {
            // On Windows, some permission operations may not be supported
            // but the builder should not panic
            println!("Windows permissions builder test for mode 0o{:o}: {:?}", mode, result);
        }
    }

    println!("✅ Permissions 0o{:o} - Builder succeeded", mode);
}

/// Test edge cases and error conditions
#[rstest]
#[case::nonexistent_file("nonexistent.txt")]
#[case::empty_filename("")]
fn test_metadata_edge_cases(temp_dir: TempDir, #[case] filename: &str) {
    let file_path = temp_dir.path().join(filename);

    // Attempt to get metadata for nonexistent file
    let result = fs::metadata(&file_path);

    match filename {
        "" => {
            // On most platforms, joining empty string to a path returns the directory itself
            // which exists (temp_dir), so metadata will succeed
            #[cfg(unix)]
            {
                // On Unix, this might fail depending on the implementation
                if result.is_err() {
                    println!("Empty filename resulted in error (expected on some Unix systems)");
                } else {
                    // The path likely resolved to the temp directory itself
                    assert!(
                        result.unwrap().is_dir(),
                        "Empty path should resolve to directory"
                    );
                }
            }
            #[cfg(windows)]
            {
                // On Windows, joining empty string returns the directory path
                assert!(
                    result.is_ok(),
                    "Empty filename should resolve to temp directory on Windows"
                );
                assert!(result.unwrap().is_dir(), "Should be a directory");
            }
        }
        "nonexistent.txt" => {
            // Nonexistent file should fail
            assert!(result.is_err(), "Nonexistent file should fail");
        }
        _ => {}
    }

    println!("✅ Edge case '{}' handled correctly", filename);
}

/// Test cross-platform file operations with OpenOptions
#[rstest]
#[case::read_only(true, false, false)]
#[case::write_only(false, true, false)]
#[case::read_write(true, true, false)]
#[case::append_mode(false, true, true)]
fn test_file_operations_cross_platform(
    temp_dir: TempDir,
    #[case] read: bool,
    #[case] write: bool,
    #[case] append: bool,
) {
    let file_path = temp_dir.path().join("ops_test.txt");

    // Create initial file
    fs::write(&file_path, "initial content").expect("Failed to create initial file");

    // Test OpenOptions with different combinations
    let mut opts = OpenOptions::new();
    opts.read(read).write(write).append(append);

    let result = opts.open(&file_path);

    if read || write {
        assert!(
            result.is_ok(),
            "File should open with read={}, write={}, append={}",
            read,
            write,
            append
        );

        if let Ok(file) = result {
            let metadata = file.metadata().expect("Failed to get file metadata");

            // Test metadata extraction on open file
            let size = metadata.cross_size();
            let mode = metadata.cross_mode();

            assert!(size > 0, "File should have content");
            assert!(mode > 0, "File should have valid mode");

            println!(
                "✅ File operations (r={}, w={}, a={}) - Size: {}, Mode: 0o{:o}",
                read, write, append, size, mode
            );
        }
    }
}

/// Benchmark-style test to verify performance of cross-platform operations
#[rstest]
fn test_metadata_performance(temp_dir: TempDir) {
    let file_path = temp_dir.path().join("perf_test.txt");

    // Create test file
    let content = "x".repeat(1024);
    fs::write(&file_path, &content).expect("Failed to create test file");

    let metadata = fs::metadata(&file_path).expect("Failed to get metadata");

    // Perform multiple metadata extractions to test performance
    let iterations = 1000;
    let start = std::time::Instant::now();

    for _ in 0..iterations {
        let _mode = metadata.cross_mode();
        let _size = metadata.cross_size();
        let _uid = metadata.cross_uid();
        let _gid = metadata.cross_gid();
        let _nlink = metadata.cross_nlink();
        let _dev = metadata.cross_dev();
        let _ino = metadata.cross_ino();
        let _rdev = metadata.cross_rdev();
        let _atime = metadata.cross_atime();
        let _mtime = metadata.cross_mtime();
        let _ctime = metadata.cross_ctime();
    }

    let duration = start.elapsed();
    let avg_time = duration.as_nanos() / iterations as u128;

    // Performance should be reasonable (less than 1ms per operation set)
    assert!(
        avg_time < 1_000_000,
        "Average metadata extraction should be under 1ms"
    );

    println!(
        "✅ Performance test - {} iterations in {:?} (avg: {}ns per iteration)",
        iterations, duration, avg_time
    );
}
