## 2024-02-23 - Path Traversal in File Receiver
**Vulnerability:** The application blindly trusted filenames provided by remote peers when creating files on the local disk. A malicious peer could send a filename like `../../etc/passwd` to overwrite arbitrary files outside the download directory.
**Learning:** `PathBuf::join` does not sanitize paths or prevent directory traversal; it merely appends components. The operating system resolves `..` components during file creation.
**Prevention:** Always sanitize user-provided filenames before using them in file system operations. In Rust, `Path::new(filename).file_name()` extracts just the filename component, stripping any directory path information.
