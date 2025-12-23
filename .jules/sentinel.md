## 2024-04-12 - Path Traversal in File Receiver
**Vulnerability:** The file receiver blindly trusted the `file_name` provided by the remote peer.
**Learning:** `PathBuf::join` replaces the base path if the joined path is absolute, and allows traversal up the directory tree if the joined path contains `..`.
**Prevention:** Always sanitize paths from untrusted sources. Use `Path::file_name()` to extract just the filename component before joining with a base directory.
