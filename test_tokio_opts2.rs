use std::os::unix::fs::OpenOptionsExt;
fn main() {
    let mut options = std::fs::OpenOptions::new();
    options.mode(0o600);
}
