//! Tests for FileInfo struct.

use p2p_core::FileInfo;
use std::path::PathBuf;

#[test]
fn test_file_info_creation() {
    let file_info = FileInfo {
        file_name: "document.pdf".to_string(),
        file_size: 1024 * 1024, // 1MB
        file_path: PathBuf::from("/home/user/document.pdf"),
    };

    assert_eq!(file_info.file_name, "document.pdf");
    assert_eq!(file_info.file_size, 1024 * 1024);
    assert_eq!(
        file_info.file_path,
        PathBuf::from("/home/user/document.pdf")
    );
}

#[test]
fn test_file_info_serialize_skips_path() {
    let file_info = FileInfo {
        file_name: "test.txt".to_string(),
        file_size: 500,
        file_path: PathBuf::from("C:\\secret\\path\\test.txt"),
    };

    let json = serde_json::to_string(&file_info).expect("Should serialize");

    // file_path should be skipped
    assert!(!json.contains("secret"));
    assert!(!json.contains("file_path"));

    // But file_name and file_size should be present
    assert!(json.contains("test.txt"));
    assert!(json.contains("500"));
}

#[test]
fn test_file_info_deserialize_without_path() {
    let json = r#"{"file_name":"received.zip","file_size":2048}"#;
    let file_info: FileInfo = serde_json::from_str(json).expect("Should deserialize");

    assert_eq!(file_info.file_name, "received.zip");
    assert_eq!(file_info.file_size, 2048);
    // file_path should be default (empty)
    assert_eq!(file_info.file_path, PathBuf::new());
}

#[test]
fn test_file_info_clone() {
    let original = FileInfo {
        file_name: "clone_test.dat".to_string(),
        file_size: 999,
        file_path: PathBuf::from("/tmp/clone_test.dat"),
    };

    let cloned = original.clone();

    assert_eq!(cloned.file_name, original.file_name);
    assert_eq!(cloned.file_size, original.file_size);
    assert_eq!(cloned.file_path, original.file_path);
}

#[test]
fn test_file_info_large_file_size() {
    let file_info = FileInfo {
        file_name: "large_file.iso".to_string(),
        file_size: 10 * 1024 * 1024 * 1024, // 10GB
        file_path: PathBuf::new(),
    };

    let json = serde_json::to_string(&file_info).expect("Should serialize large file");
    let deserialized: FileInfo = serde_json::from_str(&json).expect("Should deserialize");

    assert_eq!(deserialized.file_size, 10 * 1024 * 1024 * 1024);
}
