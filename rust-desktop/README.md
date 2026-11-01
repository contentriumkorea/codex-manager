# Codex Manager desktop development

The application entry point is `src/main.rs`; the UI is embedded from `ui/` and calls the Rust service through Tauri commands. Project, conversation and filesystem operations live in `../rust-core`.

Use `cargo test --manifest-path rust-desktop/Cargo.toml` and `node --test rust-desktop/tests/*.test.mjs` for focused checks. Packaged builds use the verified complete fixed runtime and protocol-1 assets described in [release maintenance](../docs/releasing.md).

For native UI checks, use a fresh isolated fixture and headless semantic automation. `CODEX_MANAGER_TEST_ROOT` restricts stores, workspaces and outputs to that root and prevents the test window from taking focus. Never point a test at the real Codex home. The optional `rust-core/examples/seed_ui.rs` creates fictional projects and one portable backup under a fresh `CODEX_TEST_HOME`.
