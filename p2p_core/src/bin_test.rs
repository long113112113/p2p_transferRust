use std::os::unix::fs::OpenOptionsExt;
#[tokio::main]
async fn main() {
    let mut options = tokio::fs::OpenOptions::new();
    options.mode(0o600);
}
