use p2p_core::identity::IdentityManager;
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let dir = std::env::temp_dir().join("p2p_identity_test");
    std::fs::create_dir_all(&dir).unwrap();
    let mgr = IdentityManager::new(dir.clone());
    let key = mgr.load_or_generate().await.unwrap();

    // Check perms
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(mgr.get_key_path()).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }

    // Run sync
    let key2 = mgr.load_or_generate_sync().unwrap();
    assert_eq!(key.to_bytes(), key2.to_bytes());

    std::fs::remove_dir_all(&dir).unwrap();
    println!("Test passed");
}
