use p2p_core::identity::IdentityManager;

#[tokio::test]
async fn test_perms() {
    let dir = std::env::temp_dir().join("p2p_identity_test");
    std::fs::create_dir_all(&dir).unwrap();
    let manager = IdentityManager::new(dir.clone());

    // Attempt generation multiple times, which shouldn't panic
    manager.load_or_generate().await.unwrap();
    manager.load_or_generate().await.unwrap();

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn test_perms_sync() {
    let dir = std::env::temp_dir().join("p2p_identity_test_sync");
    std::fs::create_dir_all(&dir).unwrap();
    let manager = IdentityManager::new(dir.clone());

    // Attempt generation multiple times, which shouldn't panic
    manager.load_or_generate_sync().unwrap();
    manager.load_or_generate_sync().unwrap();

    std::fs::remove_dir_all(&dir).unwrap();
}
