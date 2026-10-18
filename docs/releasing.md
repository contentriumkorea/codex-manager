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
.venv/Scripts/python.exe -m pytest -q
.venv/Scripts/python.exe -m PyInstaller --noconfirm --clean --distpath .test-artifacts/build-dist packaging/project-manager.spec
.venv/Scripts/python.exe tools/build_release.py --dist .test-artifacts/build-dist/CodexManager
.venv/Scripts/python.exe tools/verify_portable.py --release releases/v0.10.4 --root .test-artifacts/portable-0.10.4
.venv/Scripts/python.exe tools/verify_bootstrap.py --compiler "C:/Program Files (x86)/Inno Setup 6/ISCC.exe" --release releases/v0.10.4 --root .test-artifacts/bootstrap-0.10.4
```

Do not build over an installed app in `dist/CodexManager`. Keep local build outputs separate from the user's active program and state. A production bootstrap is reused from its public release; only the separately named QA installer is recompiled for validation.
