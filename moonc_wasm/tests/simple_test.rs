//! Simple test to verify basic functionality

use moonc_wasm::cross_platform::CrossPlatformMetadataExt;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_basic_functionality() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let file_path = temp_dir.path().join("test.txt");
    
    fs::write(&file_path, "test content").expect("Failed to write file");
    let metadata = fs::metadata(&file_path).expect("Failed to get metadata");
    
    let size = metadata.cross_size();
    assert_eq!(size, 12); // "test content" is 12 bytes
    
    println!("✅ Basic cross-platform test passed - Size: {}", size);
}

#[test]
fn test_platform_constants() {
    use moonc_wasm::cross_platform::platform_constants;
    
    // Just verify constants are defined
    let _nonblock = platform_constants::O_NONBLOCK;
    let _noctty = platform_constants::O_NOCTTY;
    
    println!("✅ Platform constants are accessible");
}
