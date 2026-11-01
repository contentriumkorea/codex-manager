# Windows release maintenance

The public download is `installer-v1/Codex-Manager-Setup.exe` (budget: 5 MiB). It contains the installation engine and branding, not an application payload or app version. Do not rebuild or replace it for application releases.

Application tags `vX.Y.Z` contain `Codex-Manager-Windows-x64.zip`, `update.json`, `install.ini`, and `SHA256SUMS.txt`. Both the existing updater and online installer consume the same ZIP. A byte-identical copy of the fixed installer preserves older `/releases/latest/download/Codex-Manager-Setup.exe` links; it is reused, never rebuilt. Its checksum is in the canonical installer release. Mark the app release as latest; never mark the installer release as latest. GitHub's generated source archives are not installers.

The bootstrap reads `/releases/latest/download/install.ini`, validates protocol 1, version, archive size, expanded size, and SHA-256, and constructs a URL under this repository's exact version tag. It verifies the download and extracts into a temporary directory before installation. Only QA builds accept a compile-time loopback channel; there is no runtime URL override. Download/extraction failures leave the previous installation untouched. Inno Setup handles file-copy rollback. A running or locked app blocks installation rather than being forcibly terminated.

For each app release:

1. Update `src/project_manager/version.py`, `pyproject.toml`, and the version in `README.md` / `docs/images/badge-release.svg`; add `docs/release-X.Y.Z.md` with the fixed installer link. Preserve the Contentrium README template structure and verify the rendered page after editing it.
2. Commit to `main`. The Windows release workflow runs tests, builds the app, validates metadata, and exercises native installation, failure preservation, in-app update, startup, and removal with isolated data.
3. The publisher creates draft releases before publishing. The first run creates the installer release; subsequent runs download and verify that same installer, without recompiling or editing its release. Published app versions cannot be overwritten; bump the version for a changed payload.
4. The publisher records the previous Latest release in `publication-before.json`, then anonymously downloads the public assets, checks all payload hashes and the fixed installer/legacy alias, launches a freshly extracted portable ZIP, and runs online install/start/update/uninstall through a separately registered QA installer using the real public channel. A failure after newly publishing the app restores the previous Latest, returns the failed app release to Draft, and checks the public update entry point. Never delete releases or assets as a recovery action. A saved installer requires internet access and GitHub availability.

Before publication, portable startup is also tested without installed language runtimes on PATH. Its isolated settings remain separate from the program and the check compares permanent installation, shortcuts, startup entries, and service registration before and after launch. Public verification is under `releases/public-verification`; the portable and online-install subdirectories contain native reports.

Older stable releases should become Draft only after verifying that shipped clients and supported documentation no longer depend on their versioned asset URLs. The current updater and bootstrap hold exact version URLs between discovery and download without an expiry handshake; those dependencies cannot be safely retired yet. The 0.10.4 audit therefore retains the previous application releases as a documented compatibility exception. `installer-v1` is a separate fixed distribution channel and always stays public. See [release audit](release-audit-0.10.4.md).

The release workflow uses pinned build dependencies and the Inno Setup 6.7.3 compiler with a pinned SHA-256. It uses only the repository's temporary Actions token with `contents: write`; no personal token is required. App drafts resume only for the same source commit. An unpublished installer draft can be retargeted after a workflow fix; a published installer is always reused. If publication stops after making the app public, a retry verifies that exact commit's published payload and finishes the installer without overwriting the app. Publishing needs GitHub Actions enabled and workflow/content write access when updating the source workflow.

Protocol 1 must remain backward compatible so previously saved installers continue working. If a security fix requires changing the bootstrap itself, publish a new installer generation deliberately, document the need to download it, and change the fixed public link. Routine app changes do not require this.

Optional offline developer builds: `tools/build_setup.py --compiler ISCC.exe --dist dist/CodexManager --output releases/offline` creates `Codex-Manager-Offline-Setup.exe` from an already verified payload. This is version-specific and is not the public online installer.

Native QA: `tools/verify_bootstrap.py --compiler ISCC.exe --root .test-artifacts/unique-run --release releases/vX.Y.Z`. The QA installer uses a separate app ID and a local fixture server; two channel versions use the exact same executable. Reports and logs stay under the chosen test root. The actual app startup/update test uses isolated Codex and Manager state.

Local development and verification (use a fresh output directory for every run):

```powershell
cargo test --locked --manifest-path rust-core/Cargo.toml
cargo test --locked --manifest-path rust-desktop/Cargo.toml
cargo clippy --locked --manifest-path rust-desktop/Cargo.toml --all-targets -- -D warnings
node --test rust-desktop/tests/*.test.mjs
python tools/build_rust_release.py --cab <verified-microsoft-cab> --runtime <complete-extracted-runtime> --output .test-artifacts/native-build
python tools/verify_portable.py --release .test-artifacts/native-build/app --root .test-artifacts/portable-new
python tools/verify_update.py --release .test-artifacts/native-build/app --root .test-artifacts/update-new
python tools/verify_bootstrap.py --compiler <ISCC.exe> --release .test-artifacts/native-build/app --root .test-artifacts/bootstrap-new
```

The application entry point is `rust-desktop`, with project, conversation and filesystem operations in `rust-core`. Python files remain as compatibility fixtures and release/installer validation tools; they are not packaged as the application. Do not re-enable the retired PyInstaller application build. Core and desktop Cargo versions, Tauri configuration, compatibility package version and release metadata must agree.

The native build links the MSVC runtime statically. The complete fixed WebView2 runtime is obtained from the official Microsoft CAB, checked against the pinned CAB SHA-256, independently extracted and compared file-for-file before and after staging. Microsoft executable signature validation, unchanged third-party notices and the fixed-runtime license are required. The runtime is under `_internal/WebView2` so the existing uninstaller owns it. Windows 10's AppContainer read/execute rights are applied only to that runtime directory before starting the WebView. Hardware coverage limitations remain explicit in compatibility notes.

The first 0.10.4-to-0.11.0 transition must separately exercise the actual old launcher: it has a 20-second helper-ready deadline. All payload files, including the runtime, are rehashed before ready. A new Rust timeout does not replace this measurement. A successful application health record requires frontend readiness and verified per-window taskbar properties.
