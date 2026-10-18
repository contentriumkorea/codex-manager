# Codex Manager 0.10.4 verification

Verified on 2026-10-18. Public release: [v0.10.4](https://github.com/contentriumkorea/codex-manager/releases/tag/v0.10.4).

- Application source commit: `64bcead3d6f2e3cf15b7f9159e4525180d336bfe`.
- Successful Windows CI run: [39120001617](https://github.com/contentriumkorea/codex-manager/actions/runs/39120001617).
- Later documentation-only commits do not change the published application.

## Completed checks

| Check | Result |
| --- | --- |
| Full regression on Windows CI | 272 passed, 7 skipped; skipped native HWND cases covered below |
| Native taskbar and progress suite | 11 passed, including 255/256/258 UTF-16 unit boundaries and emoji paths |
| Online installer fixtures | 14 scenarios passed: first install, same installer/new version, invalid metadata/hash/archive, missing/truncated payload, locked executable, unrelated and linked folders |
| Native installation lifecycle | Real install, Start Menu, startup, update, uninstaller preservation, removal, and retained settings/user files passed |
| Fresh portable ZIP | Direct launch, taskbar identity, isolated settings, and unchanged permanent installation/startup/service registration passed with installed language runtimes removed from PATH |
| Anonymous public downloads | ZIP, complete manifest/file hashes, update.json, install.ini, SHA256SUMS, fixed installer, and legacy latest-download installer alias passed |
| Existing updater | A client reporting 0.2.0 discovered 0.10.4 |
| Local verification of public assets | Fresh portable launch and real public-channel install/update/uninstall passed again on the local Windows machine |
| Existing local installation | Upgraded 0.10.3 to 0.10.4 using the public fixed installer; all installed payload hashes, native startup, taskbar, Start Menu, and Windows removal registration passed |
| Existing local user state | Settings retained and operation-journal hash unchanged; previous program copy preserved locally |
| README | Updated Contentrium template, local links and SVG XML checked; GitHub-rendered HTML inspected in Aside at desktop and 390-pixel content widths, including separate lower-section previews; public README blob matched the reviewed source |

The first CI attempt stopped before publication because its WScript-based shortcut reader returned an empty target for Korean filenames on English Windows. The same failure was reproduced locally with an emoji filename. Replacing the verifier with the Unicode Windows shell reader resolved it; the successful run above includes that correction.

## Public artifacts

The fixed installer was reused byte-for-byte. A saved copy continues to fetch the current application; it does not need replacement for this release.

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `installer-v1/Codex-Manager-Setup.exe` | 2,623,477 | `a083b47cc506c1df3d09e06a74190edd0d273f343800527a9dd2b120c5c34a6a` |
| `v0.10.4/Codex-Manager-Windows-x64.zip` | 49,579,205 | `67e1979947aa11d7b58e39a779f1eb83eb5502e254e51da41ea4f35f1b781324` |

[Install](https://github.com/contentriumkorea/codex-manager/releases/download/installer-v1/Codex-Manager-Setup.exe) · [Portable ZIP](https://github.com/contentriumkorea/codex-manager/releases/download/v0.10.4/Codex-Manager-Windows-x64.zip)

## Privacy, retention, and limits

Before the documentation follow-up, 396 existing Git blobs and 150 working-tree files were scanned for common credential and private-key patterns without printing values; no matches were found. The payload validator separately checked privacy exclusions, safe archive paths, the exact manifest file set, and every file hash. This is a scoped distribution check, not a formal security audit.

The public inventory now contains 15 application releases and the separate fixed installer. Previous releases remain public because shipped clients can retain their exact download URLs; see the [release-retention decision](release-audit-0.10.4.md#release-retention-decision). No releases, tags, or assets were deleted.

Project migration tests use isolated repositories. Actual-account continuation on a second physical PC and mobile Codex display remain outside this verification. Existing project formats and user conversations were not migrated or rewritten by this installation.

Detailed local evidence is under `.test-artifacts/public-release-0.10.4/verification.json`, `.test-artifacts/public-release-0.10.4/online-install/verification.json`, and `.test-artifacts/installed-user-setup-0.10.4.json`; these local reports are excluded from publication.
