## 2024-05-23 - Predictable RNG in Pairing
**Vulnerability:** The verification code generation used `SystemTime` as a seed, making it predictable.
**Learning:** Avoid using `SystemTime` or `UNIX_EPOCH` for seeding any security-relevant values. Even if the output is short (4 digits), the seed determines the sequence.
**Prevention:** Use a CSPRNG. In this codebase, `uuid` (v4) is an available source of entropy if `rand` is not directly available.
