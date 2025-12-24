## 2024-05-24 - Path Traversal in File Receiver
**Vulnerability:** The file receiver blindly trusted the `file_name` from the sender, allowing malicious peers to write files outside the download directory using path traversal sequences (e.g., `../../etc/passwd`).
**Learning:** Never trust file paths provided by external inputs. `PathBuf::join` does not sanitize or normalize paths to prevent traversal.
**Prevention:** Always sanitize filenames using `Path::file_name()` to strip directory components before joining them to a base directory.
