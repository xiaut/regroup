# TODO

- [x] Document the PATH install trick for testing `receive-*` over SSH (README.md → Building → "Install on PATH")
- [x] AGENTS.md: remind to install the branch-under-test binary on PATH when tests invoke `receive-*` commands (§9)
- [ ] install-key-identity-file: after `receive-install-key` (src/cli/receive_install_key.rs) installs the source public key on the target, syncing still prompts for a password — the `Host` block written by src/ssh/config.rs sets only `HostName`/`Port`, never `User`/`IdentityFile`, so ssh does not offer the per-connection private key (src/cli/key_management.rs); add `User` + `IdentityFile` to the Host block (and verify end-to-end on a real device per §9). Implement under AGENTS.md §1–§4 (worktree under `.worktrees/`, branch, draft PR on first commit) and remove this entry in the same PR

