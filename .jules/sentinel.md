## 2024-05-23 - Predictable RNG in Pairing
**Vulnerability:** The verification code generation used `SystemTime` as a seed, making it predictable.
**Learning:** Avoid using `SystemTime` or `UNIX_EPOCH` for seeding any security-relevant values. Even if the output is short (4 digits), the seed determines the sequence.
**Prevention:** Use a CSPRNG. In this codebase, `uuid` (v4) is an available source of entropy if `rand` is not directly available.

## 2026-01-03 - Removed Permissive CORS
**Vulnerability:** The HTTP server was configured with `CorsLayer::new().allow_origin(Any)`, allowing any website to access the internal API via the user's browser.
**Learning:** Internal tools often copy-paste permissive CORS configs from tutorials. Since the frontend is embedded and served from the same origin, CORS is unnecessary.
**Prevention:** Start with NO CORS configuration (Secure by Default). Only add it if there is a specific need for cross-origin access.
