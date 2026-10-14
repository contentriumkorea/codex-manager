# Windows release maintenance

The public download is `installer-v1/Codex-Manager-Setup.exe` (budget: 5 MiB). It contains the installation engine and branding, not an application payload or app version. Do not rebuild or replace it for application releases.

Application tags `vX.Y.Z` contain `Codex-Manager-Windows-x64.zip`, `update.json`, `install.ini`, and `SHA256SUMS.txt`. Both the existing updater and online installer consume the same ZIP. Mark the app release as latest; never mark the installer release as latest. GitHub's generated source archives are not installers.

The bootstrap reads `/releases/latest/download/install.ini`, validates protocol 1, version, archive size, expanded size, and SHA-256, and constructs a URL under this repository's exact version tag. It verifies the download and extracts into a temporary directory before installation. Only QA builds accept a compile-time loopback channel; there is no runtime URL override. Download/extraction failures leave the previous installation untouched. Inno Setup handles file-copy rollback. A running or locked app blocks installation rather than being forcibly terminated.

For each app release:

1. Update `src/project_manager/version.py` and `pyproject.toml`; add `docs/release-X.Y.Z.md` with the fixed installer link.
2. Commit to `main`. The Windows release workflow runs tests, builds the app, validates metadata, and exercises native installation, failure preservation, in-app update, startup, and removal with isolated data.
3. The publisher creates draft releases before publishing. The first run creates the installer release; subsequent runs download and verify that same installer, without recompiling or editing its release. Published app versions cannot be overwritten; bump the version for a changed payload.
4. Verify the public fixed installer link and the app's update discovery after publication. Test installation from the public channel. A saved installer requires internet access and GitHub availability.

The release workflow uses pinned build dependencies and the Inno Setup 6.7.3 compiler with a pinned SHA-256. It uses only the repository's temporary Actions token with `contents: write`; no personal token is required. Draft uploads can resume only for the same source commit. Publishing needs GitHub Actions enabled and workflow/content write access when updating the source workflow.

Protocol 1 must remain backward compatible so previously saved installers continue working. If a security fix requires changing the bootstrap itself, publish a new installer generation deliberately, document the need to download it, and change the fixed public link. Routine app changes do not require this.

Optional offline developer builds: `tools/build_setup.py --compiler ISCC.exe --dist dist/CodexManager --output releases/offline` creates `Codex-Manager-Offline-Setup.exe` from an already verified payload. This is version-specific and is not the public online installer.

Native QA: `tools/verify_bootstrap.py --compiler ISCC.exe --root .test-artifacts/unique-run --release releases/vX.Y.Z`. The QA installer uses a separate app ID and a local fixture server; two channel versions use the exact same executable. Reports and logs stay under the chosen test root. The actual app startup/update test uses isolated Codex and Manager state.
