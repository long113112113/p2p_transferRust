## 2024-05-23 - Rust Path Sanitization Pitfalls
**Vulnerability:** Relied on `std::path::Path::new(path).file_name()` for filename sanitization in a cross-platform context.
**Learning:** `Path::new` behavior is OS-dependent. On Linux, `\` is treated as a valid filename character, not a separator. If a Windows client sends `..\..\evil.exe`, a Linux server might accept it as a filename containing backslashes, or fail to strip the directory traversal components if it expected `/`. Conversely, web inputs might mix separators.
**Prevention:** For network inputs, always manually sanitize filenames by treating both `/` and `\` as separators and filtering control characters, rather than relying on the host OS's `Path` implementation.
