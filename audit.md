# Audit Report: Regroup

## Executive Summary

**Project**: Automatic machine discovery tool on local networks via signed UDP beacons, with decentralized Git synchronization.
**Language**: Rust (edition 2024)
**Version**: 0.2.0
**Status**: Functional, well-structured, with identified improvement areas.

---

## Strengths

### 1. Architecture & Design
- **Excellent modularity**: Clear separation of concerns (CLI, config, discovery, git, signing, ssh, tui)
- **Security by design**:
  - Machine identification by **SSH server public key** (not hostname/IP)
  - Ed25519 beacon signing with systematic verification
  - Dual anti-MITM verification (beacon signature + ssh-keyscan)
  - Signing key pinning in configuration files
- **Decentralized synchronization**: Smart git architecture with last-writer-wins conflict resolution
- **Strict ownership**: Each machine writes only in its own directory

### 2. Code Quality
- **Comprehensive unit tests**: 25 passing tests covering signing, config, CLI
- **Documentation**: Detailed comments, docstrings, complete README
- **Error handling**: Mostly robust, with proper error propagation
- **Strong typing**: Wrapper types to prevent confusion (SshPublicKey, SigningPublicKeyB64, etc.)

### 3. Security
- **Beacon verification**: Age, signature, IP consistency, clock skew checks
- **SSH verification**: Fingerprint comparison before adding devices
- **Permissions**: Sensitive files (private keys) set to 0600
- **Isolation**: Signing keys stored outside git repository

### 4. Maintenance
- **Clean build**: Compiles without warnings in release mode
- **Clippy**: Only 2 minor warnings (consecutive string replaces)
- **Cargo.toml**: Well-chosen, up-to-date dependencies

---

## Issues and Recommendations

---

### Critical

#### 1. Security - File Permissions
**Issue**: File permissions are not consistently enforced on configuration files.
- `save_device_file` does not apply restrictive permissions
- `~/.regroup-config/git/` directory could be created with overly permissive settings

**Recommendation**:
```rust
// In src/config/device.rs and loading.rs
use std::os::unix::fs::PermissionsExt;
fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
```

#### 2. Security - Input Validation
**Issue**: Some validations are incomplete:
- `create_device_config` accepts potentially dangerous hostnames
- SSH key format validation is only presence-based, not format-based

**Recommendation**:
- Add regex validation for SSH keys (OpenSSH format)
- Reject device names containing special characters (`/`, `\`, etc.)

#### 3. Security - Secrets in Memory
**Issue**: Private signing keys are loaded in memory without systematic zeroization.

**Recommendation**:
- Use `Zeroizing` for keys in memory (already partially present in signing.rs)
- Implement `Drop` trait to clear memory after use

---

### Important

#### 4. Code Quality - Error Handling
**Issue**: 129 occurrences of `unwrap_or`/`unwrap()`/`expect()` in the codebase, including production code.

**Critical locations**:
- `src/config/loading.rs`: `tracker.lock().unwrap()` → Deadlock risk
- `src/ssh/config.rs`: `fs::read_to_string(path).unwrap_or_default()` → Hides errors
- `src/discovery/local.rs`: Multiple `unwrap_or_default()` on system commands

**Recommendation**:
- Replace with explicit error handling using `?`
- Use `expect_with_context` or custom errors
- For locks: use `lock().expect("mutex poisoned")`

#### 5. Maintenance - Dependencies
**Issue**:
- **ed25519-dalek v2.2.0**: Recent version, needs vulnerability check
- **rand_core v0.6.4**: Indirect dependency, verify trust chain
- No `cargo-audit` configured

**Recommendation**:
- Add `cargo-audit` to CI workflow
- Check advisories for ed25519-dalek and rand_core
- Regularly update `Cargo.lock`

#### 6. Maintenance - Rust Edition
**Issue**: Using **Rust edition 2024** which may not be stable in all environments (especially Termux).

**Recommendation**:
- Revert to **edition 2021** for better compatibility
- Or explicitly document minimum Rust version requirement

#### 7. Configuration - Default Files
**Issue**:
- `settings.json` is auto-created but not strictly validated
- No JSON schema validation for config files

**Recommendation**:
- Add JSON schema validation with `jsonschema` or strict manual validation
- Handle unknown fields (ignore or reject)

---

### Nice to Have

#### 8. Tests - Coverage
**Issue**: Integration tests are missing; only unit tests present.

**Recommendation**:
- Add integration tests for:
  - Complete workflow: beacon → add → git sync
  - Name conflict detection
  - Permission management
- Use `cargo test -- --nocapture` to see logs

#### 9. Documentation
**Issue**:
- README is comprehensive but some sections need updates
- No generated documentation (`cargo doc`)

**Recommendation**:
- Generate and publish documentation with `cargo doc --open`
- Add examples in public function docs
- Document possible error returns

#### 10. CI/CD
**Issue**: No `.github/workflows/` files visible.

**Recommendation**:
- Add GitHub Actions workflow for:
  - Build and test on Linux
  - Clippy checks
  - Security audit (cargo-audit)
  - Termux build (cross-compilation)

#### 11. Performance
**Issue**:
- `listen_beacons_continuous` uses a fixed 4096-byte buffer
- No explicit rate limiting on incoming beacons

**Recommendation**:
- Implement rate limiting per source IP
- Handle oversized beacons
- Use dynamic buffer or validate size

#### 12. User Experience
**Issue**:
- Warning/error messages sometimes unclear
- No persistent logging (only in-memory)

**Recommendation**:
- Add optional log file (`--log-file`)
- Structure messages with levels (DEBUG, INFO, WARN, ERROR)
- Use a crate like `tracing` or `log`

#### 13. Compatibility
**Issue**:
- Implicit dependency on `ssh-keyscan` and `ssh-keygen`
- No fallback for systems without these tools

**Recommendation**:
- Check tools at startup with clear messages
- Document prerequisites per platform
- Add alternatives (e.g., direct SSH file reading)

---

## Security Checklist

| Category | Status | Details |
|----------|--------|---------|
| **Authentication** | Good | Ed25519 + SSH host key verification |
| **Confidentiality** | Medium | Private keys at 0600, but incomplete zeroization |
| **Integrity** | Good | Verified signatures, optional signed git commits |
| **Availability** | Good | Decentralized architecture, fault-tolerant |
| **Input Validation** | Medium | Present but incomplete |
| **Error Handling** | Medium | Robust but with remaining `unwrap` calls |
| **Permissions** | Medium | Partially applied |
| **Updates** | Medium | No automatic update mechanism |

---

## Action Plan

### Immediate (Before Release)
1. Fix file permissions on all sensitive files
2. Improve validation for SSH keys and device names
3. Replace critical `unwrap()` calls with proper error handling

### Short Term (1-2 weeks)
4. Add cargo-audit to workflow
5. Set up CI/CD with tests and checks
6. Document Rust 2024 compatibility

### Medium Term (1 month)
7. Add integration tests
8. Improve logging
9. Update README with advanced use cases

---

## Metrics

| Metric | Value | Target |
|--------|-------|--------|
| Lines of Rust code | ~2,500 | - |
| Unit tests | 25 | >= 30 |
| Test coverage | ~80% (estimated) | >= 85% |
| Clippy warnings | 2 | 0 |
| Direct dependencies | 11 | - |
| Release build time | ~20s | < 30s |
| Release binary size | ~5MB | < 10MB |

---

## Conclusion

The **regroup** project is **well-designed and secure** overall. The decentralized architecture with SSH key-based identification and Ed25519 signatures provides a solid foundation for network machine discovery.

**Major strengths**: Security by design, modularity, comprehensive tests.

**Priority fixes**: Permission management, input validation, removal of critical `unwrap()` calls.

**Final recommendation**: The project can be considered **ready for version 0.3.0** after addressing the identified critical issues. A 1.0.0 release would require adding integration tests and improved error handling.
