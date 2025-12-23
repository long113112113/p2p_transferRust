use std::path::PathBuf;

#[test]
fn test_path_sanitization_logic() {
    let download_dir = PathBuf::from("/tmp/downloads");

    // Case 1: Traversal
    let malicious_name_1 = "../../etc/passwd";
    let safe_name_1 = std::path::Path::new(malicious_name_1)
        .file_name()
        .unwrap();
    let path_1 = download_dir.join(safe_name_1);
    assert_eq!(path_1, PathBuf::from("/tmp/downloads/passwd"));

    // Case 2: Absolute path
    let malicious_name_2 = "/etc/passwd";
    let safe_name_2 = std::path::Path::new(malicious_name_2)
        .file_name()
        .unwrap();
    let path_2 = download_dir.join(safe_name_2);
    assert_eq!(path_2, PathBuf::from("/tmp/downloads/passwd"));

    // Case 3: Normal file
    let normal_name = "cool_file.txt";
    let safe_name_3 = std::path::Path::new(normal_name)
        .file_name()
        .unwrap();
    let path_3 = download_dir.join(safe_name_3);
    assert_eq!(path_3, PathBuf::from("/tmp/downloads/cool_file.txt"));
}
