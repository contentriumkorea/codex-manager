# 0.10.3 release verification — 2026-10-14

Public source commit: `e50a6390131ec733144286c993e097d4180d6bb5`.

[Windows release workflow](https://github.com/contentriumkorea/codex-manager/actions/runs/38540834234) completed successfully. The first attempt exposed a missing Codex runtime in the clean CI host; the workflow now provisions official Codex CLI 0.160.0 for isolated integration tests. No tests were bypassed to publish.

- CI regression: **259 passed, 2 skipped** (Windows HWND tests are skipped by the offscreen Qt test platform).
- Local native Windows shell/progress tests: **6 passed**, including both HWND tests.
- Native bootstrap acceptance: **14 fixture scenarios plus actual app installation/startup/in-app update/uninstallation**, all passed both locally and in CI. One unchanged QA installer installs two different channel versions. Invalid metadata, bad hash, unavailable/truncated download, corrupt archive, locked executable, unrelated directory and linked directory preserve the existing installation.
- Anonymous public downloads: installer, compatibility alias, ZIP, INI, JSON and checksums verified. All **219 payload file hashes** matched; ZIP CRC passed. The previous app's update checker discovers 0.10.3, and `/releases/latest` points to the app release.
- Public-channel installation in an isolated QA directory: install, native startup, updater, uninstaller, Start Menu and settings/user-file retention passed.
- User PC: the downloaded public installer replaced 0.10.2 with **0.10.3** in the existing location, with a verified copy of the previous program. Payload hashes, Start Menu target/icon, uninstall registration, taskbar identity, settings retention and unchanged operation journal passed. The upgraded app started successfully.

| Public asset | Bytes | SHA-256 |
|---|---:|---|
| [Fixed installer](https://github.com/contentriumkorea/codex-manager/releases/download/installer-v1/Codex-Manager-Setup.exe) | 2,623,477 | `a083b47cc506c1df3d09e06a74190edd0d273f343800527a9dd2b120c5c34a6a` |
| [App ZIP](https://github.com/contentriumkorea/codex-manager/releases/download/v0.10.3/Codex-Manager-Windows-x64.zip) | 49,578,610 | `43a8fb0905720938e25d5480b2c8f23734f5c3113e0d2459e8327d43546a54fc` |

Expanded payload: 124,238,882 bytes. The installer requires internet access and downloads this payload separately. The compatibility installer in the app release is byte-identical to the fixed installer, preserving previously shared download links without rebuilding the installer.

Local evidence is kept under `.test-artifacts/bootstrap-qa-final/`, `.test-artifacts/public-install-0.10.3/`, `.test-artifacts/public-release-0.10.3/public-verification.json`, and `.test-artifacts/installed-user-setup-0.10.3.json`. These reports contain local paths and are not distributed with the application. Real-account continuation and a second physical PC remain outside these isolated installer checks.
