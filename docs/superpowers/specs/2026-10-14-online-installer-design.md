# Stable online installer

The user wants one small installer that automatically installs the newest Codex Manager, with installer distribution separate from application updates. GitHub publishing and local replacement remain authorized. The saved installer must continue to work across ordinary application releases without being rebuilt.

## Design

- Keep the existing Inno Setup Windows integration, but remove the application payload from its EXE. Use Inno's native HTTPS download, SHA-256 verification and ZIP extraction, then its normal installation/uninstallation handling. A custom .NET downloader would duplicate that handling; a downloader for a versioned full installer would still require rebuilding full installers. Both are unnecessary here.
- Publish the bootstrap once under the stable `installer-v1` release. README's first link points to `releases/download/installer-v1/Codex-Manager-Setup.exe`. That release must never become the latest application release. Bootstrap size budget: 5 MiB.
- Application releases keep the existing `Codex-Manager-Windows-x64.zip` and `update.json` protocol, adding `install.ini` with protocol version, application version, archive size/SHA-256 and expanded size. The installer resolves `releases/latest/download/install.ini` and constructs the archive URL from the validated numeric version and fixed repository. No arbitrary download URL from the manifest or command line.
- Download and extract into installer temporary storage before changing the installation. A cancelled or failed download, bad hash, invalid manifest, unusable archive or running application must stop before replacement. Preserve user settings, conversations, unknown installation-root files and existing installation location. Show download and extraction progress.
- Retain the current AppId, Start menu/icon, optional desktop shortcut and Windows uninstaller. Windows DisplayVersion reflects the downloaded app version, independently of bootstrap version. Later in-app updates retain the uninstaller.
- The bootstrap itself changes only if the installer protocol/behavior needs revision. App releases update the payload and metadata, not the bootstrap.

## Verification and publication

Compile a separately named QA bootstrap restricted to a local HTTP fixture server. Run the same EXE against two channel versions and prove its hash stays identical while the installed payload/version changes. Exercise download failure, invalid metadata/hash, fresh install, reinstall, settings/unknown-file preservation, native startup, in-app update and uninstall. QA overrides must be compile-time only and absent from production.

Publish source and releases to the existing public Contentrium repository. The connected GitHub app has repository access; local Git/gh credentials currently fail. If needed, publish through a reviewed GitHub Actions workflow using repository-scoped permissions, without exporting credentials. Verify anonymous downloads and new/old update clients against the public release, then replace the local app safely.
