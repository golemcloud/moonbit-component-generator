//! Unit tests for cross-platform module components
//!
//! These tests focus on individual components and internal functionality

use rstest::rstest;
use std::fs::{self, File};
use tempfile::TempDir;

use moonc_wasm::cross_platform::{
    CrossPlatformMetadataExt, MetadataExtractor, PermissionsBuilder, platform_constants,
};

#[cfg(test)]
mod metadata_extractor_tests {
    use super::*;

    /// Test MetadataExtractor with real file metadata
    #[rstest]
    fn test_metadata_extractor_basic() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let file_path = temp_dir.path().join("test.txt");

        // Create test file with known content
        let content = "Hello, cross-platform world!";
        fs::write(&file_path, content).expect("Failed to write file");

        let metadata = fs::metadata(&file_path).expect("Failed to get metadata");

        // Test all extractor methods
        let mode = MetadataExtractor::mode(&metadata);
        let _dev = MetadataExtractor::dev(&metadata);
        let _ino = MetadataExtractor::ino(&metadata);
        let nlink = MetadataExtractor::nlink(&metadata);
        let uid = MetadataExtractor::uid(&metadata);
        let gid = MetadataExtractor::gid(&metadata);
        let _rdev = MetadataExtractor::rdev(&metadata);
        let size = MetadataExtractor::size(&metadata);
        let _atime = MetadataExtractor::atime(&metadata);
        let mtime = MetadataExtractor::mtime(&metadata);
        let _ctime = MetadataExtractor::ctime(&metadata);

        // Verify reasonable values
        assert!(mode > 0, "Mode should be positive");
        assert_eq!(
            size as usize,
            content.len(),
            "Size should match content length"
        );
        assert!(nlink >= 1, "Should have at least one link");
        assert!(mtime > 0, "Modification time should be positive");

        // Platform-specific checks
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                assert!(uid >= 0, "UID should be non-negative on Unix");
                assert!(gid >= 0, "GID should be non-negative on Unix");
                assert!(dev > 0, "Device ID should be positive on Unix");
                assert!(ino > 0, "Inode should be positive on Unix");
            } else if #[cfg(windows)] {
                assert_eq!(uid, 0, "UID should be 0 on Windows (simulated)");
                assert_eq!(gid, 0, "GID should be 0 on Windows (simulated)");
            }
        }

        println!(
            "✅ MetadataExtractor - Mode: 0o{:o}, Size: {}, Links: {}",
            mode, size, nlink
        );
    }

    /// Test metadata extraction with different file types
    #[rstest]
    #[case::regular_file("regular.txt", "Regular file content")]
    #[case::empty_file("empty.txt", "")]
    #[case::binary_file("binary.bin", "\x00\x01\x02\x03\x7F")]
    fn test_metadata_extractor_file_types(#[case] filename: &str, #[case] content: &str) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let file_path = temp_dir.path().join(filename);

        fs::write(&file_path, content).expect("Failed to write file");
        let metadata = fs::metadata(&file_path).expect("Failed to get metadata");

        // Test size extraction
        let size = MetadataExtractor::size(&metadata);
        assert_eq!(
            size as usize,
            content.len(),
            "Size should match for {}",
            filename
        );

        // Test timestamps
        let _atime = MetadataExtractor::atime(&metadata);
        let mtime = MetadataExtractor::mtime(&metadata);
        let _ctime = MetadataExtractor::ctime(&metadata);

        assert!(
            mtime > 0 || content.is_empty(),
            "MTime should be positive for non-empty files"
        );
        assert!(_ctime >= 0, "CTime should be non-negative");

        println!(
            "✅ File type {} - Size: {}, MTime: {}",
            filename, size, mtime
        );
    }
}

#[cfg(test)]
mod permissions_builder_tests {
    use super::*;

    /// Test PermissionsBuilder with standard Unix permission values
    #[rstest]
    #[case::owner_read_write(0o600, "owner read-write")]
    #[case::standard_file(0o644, "standard file permissions")]
    #[case::executable(0o755, "executable permissions")]
    #[case::world_writable(0o666, "world writable")]
    #[case::no_permissions(0o000, "no permissions")]
    #[case::all_permissions(0o777, "all permissions")]
    fn test_permissions_builder_modes(#[case] mode: u32, #[case] description: &str) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let file_path = temp_dir.path().join("perm_test.txt");

        // Create test file
        File::create(&file_path).expect("Failed to create file");

        // Test permissions builder
        let permissions = PermissionsBuilder::from_mode(mode);
        let result = fs::set_permissions(&file_path, permissions);

        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                assert!(result.is_ok(), "Setting {} (0o{:o}) should succeed on Unix", description, mode);

                // Verify permissions were applied
                let metadata = fs::metadata(&file_path).expect("Failed to get metadata");
                let actual_mode = metadata.cross_mode() & 0o777;

                // Check readonly flag for write permissions
                if mode & 0o200 == 0 {
                    assert!(metadata.permissions().readonly() || (actual_mode & 0o200) == 0,
                           "File should be readonly when write bit is cleared");
                }
            } else if #[cfg(windows)] {
                // On Windows, just verify the builder doesn't panic
                println!("Windows permissions test for {}: {:?}", description, result);
            }
        }

        println!("✅ {} (0o{:o}) - Builder completed", description, mode);
    }

    /// Test edge cases for permissions
    #[rstest]
    #[case::setuid_bit(0o4755, "setuid bit")]
    #[case::setgid_bit(0o2755, "setgid bit")]
    #[case::sticky_bit(0o1755, "sticky bit")]
    #[case::all_special_bits(0o7777, "all special bits")]
    fn test_permissions_special_bits(#[case] mode: u32, #[case] description: &str) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let file_path = temp_dir.path().join("special_perm_test.txt");

        File::create(&file_path).expect("Failed to create file");

        // Test with special permission bits
        let permissions = PermissionsBuilder::from_mode(mode);
        let result = fs::set_permissions(&file_path, permissions);

        // Special bits may not be supported on all filesystems
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                // On Unix, this might succeed or fail depending on filesystem support
                println!("Unix special permissions test for {} (0o{:o}): {:?}", description, mode, result);
            } else if #[cfg(windows)] {
                // Windows doesn't support Unix special bits
                println!("Windows special permissions test for {} (0o{:o}): {:?}", description, mode, result);
            }
        }

        println!(
            "✅ {} (0o{:o}) - Special bits test completed",
            description, mode
        );
    }
}

#[cfg(test)]
mod platform_constants_tests {
    use super::*;

    /// Test that platform constants are properly defined
    #[rstest]
    fn test_platform_constants_defined() {
        // Test that all constants are defined and have reasonable values
        let constants = [
            ("O_NONBLOCK", platform_constants::O_NONBLOCK),
            ("O_NOCTTY", platform_constants::O_NOCTTY),
            ("O_DSYNC", platform_constants::O_DSYNC),
            ("O_SYNC", platform_constants::O_SYNC),
        ];

        for (name, value) in constants {
            assert!(value >= 0, "{} should be non-negative", name);

            cfg_if::cfg_if! {
                if #[cfg(unix)] {
                    // On Unix, some constants should have specific values
                    match name {
                        "O_NONBLOCK" => assert!(value > 0, "O_NONBLOCK should be positive on Unix"),
                        "O_NOCTTY" => assert!(value >= 0, "O_NOCTTY should be non-negative on Unix"),
                        _ => {}
                    }
                } else if #[cfg(windows)] {
                    // On Windows, some constants might be 0 (unsupported)
                    println!("Windows constant {} = 0x{:x}", name, value);
                }
            }
        }

        println!("✅ All platform constants are properly defined");
    }

    /// Test constants don't conflict with each other
    #[rstest]
    fn test_constants_uniqueness() {
        let constants = vec![
            platform_constants::O_NONBLOCK,
            platform_constants::O_NOCTTY,
            platform_constants::O_DSYNC,
            platform_constants::O_SYNC,
        ];

        // Remove zeros (unsupported flags on some platforms)
        let non_zero_constants: Vec<_> = constants.into_iter().filter(|&x| x != 0).collect();

        // Check for duplicates among non-zero constants
        for (i, &const1) in non_zero_constants.iter().enumerate() {
            for &const2 in non_zero_constants.iter().skip(i + 1) {
                assert_ne!(const1, const2, "Constants should not have duplicate values");
            }
        }

        println!("✅ Platform constants are unique (excluding zeros)");
    }
}

#[cfg(test)]
mod trait_extension_tests {
    use super::*;

    /// Test CrossPlatformMetadataExt trait methods
    #[rstest]
    fn test_trait_extension_methods() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let file_path = temp_dir.path().join("trait_test.txt");

        let content = "Testing trait extension methods";
        fs::write(&file_path, content).expect("Failed to write file");

        let metadata = fs::metadata(&file_path).expect("Failed to get metadata");

        // Test all trait methods
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

        // Verify all methods return reasonable values
        assert!(mode > 0, "cross_mode should return positive value");
        assert_eq!(
            size as usize,
            content.len(),
            "cross_size should match content length"
        );
        assert!(nlink >= 1, "cross_nlink should be at least 1");
        assert!(mtime > 0, "cross_mtime should be positive");
        assert!(ctime >= 0, "cross_ctime should be non-negative");

        // Test that trait methods match direct extractor calls
        assert_eq!(
            dev,
            MetadataExtractor::dev(&metadata),
            "cross_dev should match extractor"
        );
        assert_eq!(
            ino,
            MetadataExtractor::ino(&metadata),
            "cross_ino should match extractor"
        );
        assert_eq!(
            mode,
            MetadataExtractor::mode(&metadata),
            "cross_mode should match extractor"
        );
        assert_eq!(
            nlink,
            MetadataExtractor::nlink(&metadata),
            "cross_nlink should match extractor"
        );
        assert_eq!(
            uid,
            MetadataExtractor::uid(&metadata),
            "cross_uid should match extractor"
        );
        assert_eq!(
            gid,
            MetadataExtractor::gid(&metadata),
            "cross_gid should match extractor"
        );
        assert_eq!(
            rdev,
            MetadataExtractor::rdev(&metadata),
            "cross_rdev should match extractor"
        );
        assert_eq!(
            size,
            MetadataExtractor::size(&metadata),
            "cross_size should match extractor"
        );
        assert_eq!(
            atime,
            MetadataExtractor::atime(&metadata),
            "cross_atime should match extractor"
        );
        assert_eq!(
            mtime,
            MetadataExtractor::mtime(&metadata),
            "cross_mtime should match extractor"
        );
        assert_eq!(
            ctime,
            MetadataExtractor::ctime(&metadata),
            "cross_ctime should match extractor"
        );

        println!("✅ All trait extension methods work correctly and match extractors");
    }
}
