# Security Audit Report

**Date:** 2024-05-23
**Target:** Rust Backend Web Application (p2p_core, p2p_wan, p2p_gui)
**Auditor:** Senior Security Engineer (Jules)

## Executive Summary

- **Risk Level:** **Medium-High**
- **Vulnerabilities Found:** 4
- **Security Score:** 7/10

The application demonstrates a solid foundation in security, utilizing safe Rust patterns and modern cryptographic libraries (`rustls`, `blake3`, `uuid`). Key strengths include the use of high-entropy UUIDs for session tokens and robust file path sanitization.

However, significant risks exist in the web server configuration, specifically the overly permissive CORS policy (`*`), which could expose the local server to cross-origin attacks. Additionally, the storage of sensitive identity keys lacks restrictive file permissions, and the reliance on URL-based session tokens presents a risk of token leakage.

## Critical Vulnerabilities (CVSS 9.0+)

*None identified.*

## High Risk Issues (CVSS 7.0-8.9)

### 1. Insecure CORS Configuration (Wildcard Origin)
- **Severity:** HIGH
- **CVSS:** 8.2 (CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:L/A:N)
- **Location:** `p2p_core/src/http_share/server.rs`
- **Description:** The HTTP server is configured to allow Cross-Origin Resource Sharing (CORS) from **any** origin (`Any`).
  ```rust
  let cors = CorsLayer::new()
      .allow_origin(Any) // ❌ Allows malicious sites to communicate with the server
      .allow_methods(Any)
      .allow_headers(Any);
  ```
- **Impact:** If a user visits a malicious website while running the application, that website can send requests to the local server (e.g., `localhost:8080`). While the attacker needs the session token, the permissive policy removes a critical layer of defense. If the token is ever leaked or guessable, the attacker has full access.
- **Remediation:**
  - **Restrict Origins:** If the frontend is served from the same domain/port (embedded), CORS might not be needed at all or should be restricted to `localhost` and specific paired peers.
  - **Remove CORS for API:** If the API is not intended to be accessed by third-party browsers, remove the `CorsLayer` or set it to `CorsLayer::permissive()` only in development.

  ```rust
  // Recommended Fix
  let cors = CorsLayer::new()
      // Only allow requests from the same origin or specific trusted domains
      // If the app is local-only, this might be overly permissive.
      .allow_origin(tower_http::cors::Any); // Re-evaluate if this is strictly necessary
  ```
  *Better approach:* Do not apply CORS middleware if the API is only consumed by the embedded frontend served from the same origin.

## Medium Risk Issues (CVSS 4.0-6.9)

### 2. Insecure File Permissions on Identity Key
- **Severity:** MEDIUM
- **CVSS:** 5.5 (CVSS:3.1/AV:L/AC:L/PR:L/UI:N/S:U/C:H/I:N/A:N)
- **Location:** `p2p_core/src/identity.rs`
- **Description:** The node's secret key (`node_secret.key`) is written to disk using standard `fs::write`, which typically results in default permissions (e.g., `644` or `666`). This allows other users on the same system to read the secret key.
  ```rust
  fs::write(&key_path, secret_key.to_bytes()) // ❌ Readable by others
  ```
- **Impact:** Local privilege escalation. A malicious user on the same machine can steal the node identity and impersonate the user.
- **Remediation:** Explicitly set file permissions to `600` (read/write only for owner) on Unix-like systems.

  ```rust
  // Recommended Fix
  use std::os::unix::fs::PermissionsExt;

  let mut file = File::create(&key_path).await?;
  let mut perms = file.metadata().await?.permissions();
  perms.set_mode(0o600); // ✅ Secure
  file.set_permissions(perms).await?;
  file.write_all(&secret_key.to_bytes()).await?;
  ```

### 3. Session Token Leakage via URL
- **Severity:** MEDIUM
- **CVSS:** 4.3 (CVSS:3.1/AV:N/AC:L/PR:N/UI:R/S:U/C:L/I:N/A:N)
- **Location:** `p2p_core/src/http_share/server.rs`
- **Description:** Authentication relies on the session token being present in the URL path (`/{token}/...`).
- **Impact:** URLs are frequently logged by proxies, browsers (history), and servers. If the user shares a screenshot or screen share, the token is visible.
- **Remediation:**
  - Prefer passing the token via an `Authorization` header (e.g., `Bearer <token>`) or a cookie.
  - Keep the URL token for initial handshake/pairing if necessary, but exchange it for a session cookie or header immediately.

### 4. Unsafe Memory Mapping
- **Severity:** LOW
- **CVSS:** 3.0
- **Location:** `p2p_core/src/transfer/hash.rs`
- **Description:** Usage of `unsafe { memmap2::Mmap::map(&file) }`.
- **Impact:** If the file underlying the mmap is truncated by another process, the application will crash (SIGBUS) or exhibit undefined behavior.
- **Remediation:** Ensure the file is locked or accept the risk for performance. Document the specific safety invariants.

## Security Best Practices Compliance

### Authentication ✅
- **Token Entropy:** ✅ Excellent. Uses `Uuid::new_v4()` (128-bit).
- **Password Hashing:** N/A (No user passwords).
- **Pairing Codes:** ✅ Uses `Uuid` bytes for entropy.

### Input Validation
- **SQL Injection:** ✅ N/A (No SQL DB).
- **Path Traversal:** ✅ Protected. `sanitize_file_name` strips directories.
- **File Uploads:** ⚠️ Partial.
  - ✅ Size limit enforced (10GB).
  - ✅ Filename length limit.
  - ❌ Content-Type/Magic byte validation missing.

### Secrets Management
- **Hardcoded Secrets:** ✅ None found.
- **Environment Vars:** ✅ Supported via `dotenvy`.

### Denial of Service (DoS)
- **Large Files:** ✅ Limited.
- **Timeouts:** ✅ WebSocket pings implemented.

## Remediation Roadmap

### Phase 1: Immediate (Next Sprint)
1.  **Fix File Permissions:** Update `identity.rs` to set `0o600` permissions on the secret key file.
2.  **Restrict CORS:** Analyze if `Access-Control-Allow-Origin: *` is strictly necessary. If not, remove it or restrict it to specific origins.

### Phase 2: Short-term (1 Month)
3.  **Enhance Upload Security:** Implement magic byte detection (`infer` crate) to validate file types, restricting executable uploads if not needed.
4.  **Security Headers:** Add `axum` middleware to set security headers (`X-Content-Type-Options: nosniff`, `Content-Security-Policy`).

### Phase 3: Long-term
5.  **Refactor Auth:** Move away from URL-based tokens to Header-based auth for better hygiene.
6.  **Fuzz Testing:** Run `cargo fuzz` on the file parsing logic.

## Security Testing Performed
- [x] Static Analysis (Manual Code Review)
- [x] Keyword Search (Secrets, Unsafe, SQL)
- [x] Dependency Review (`Cargo.toml`)
- [x] Architecture Review (Auth flow)
