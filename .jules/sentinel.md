## 2024-04-20 - [Weak RNG in Verification Code]
**Vulnerability:** The `generate_verification_code` function in `p2p_core/src/pairing.rs` used `SystemTime` as a seed for random number generation.
**Learning:** Even for simple 4-digit codes, using time-based seeding makes the code predictable if the approximate generation time is known.
**Prevention:** Use a Cryptographically Secure Pseudo-Random Number Generator (CSPRNG). In this case, `uuid::Uuid::new_v4()` was used as it relies on a CSPRNG and was already a dependency.
