# 0.10.4 distribution audit

Scope: apply the Contentrium development skill to this existing Windows app without changing its framework, project format, or fixed online installer.

## Checks

- Keep `installer-v1/Codex-Manager-Setup.exe` byte-identical; verify the canonical link and the legacy latest-download alias.
- Put installation and portable downloads together near the top of the README, with a project-specific cover, actual demonstration screenshot, and Contentrium homepage link.
- Verify archive paths, privacy exclusions, the complete file manifest, and all hashes before publication.
- Run the portable ZIP directly from a fresh folder with isolated data; check native startup, taskbar identity, and absence of permanent installation registration.
- Verify anonymous public downloads and installation after publishing. Record the previous release and restore it if validation of a newly published version fails.
- Run regression and native installer/update checks, then upgrade the existing user installation while preserving its settings and operation journal.

## Release retention decision

Before this release, the authenticated release inventory contained 14 application releases and the separate `installer-v1` channel. The second page was empty.

The shipped updater retains exact version URLs in its `Release` object between discovery and download, and the fixed bootstrap constructs a version URL after reading the latest channel. There is no expiry or refresh handshake that proves all existing clients have abandoned older URLs. Historical verification documentation also links to the 0.10.3 package. Drafting these releases would make those public asset URLs unavailable. Keep previous releases public under the skill's download-dependency exception; do not delete releases, assets, tags, or commits. Keep the independent installer channel public and mark only the new application release as Latest.

## Existing behavior retained

The installer provides Start Menu registration, an optional desktop shortcut, and Windows removal. The portable app uses `%LOCALAPPDATA%/ProjectConversationManager` for Manager settings; Codex data and project files remain in their existing locations. A program ZIP alone is not a project backup. No new tray, auto-start, or system service is introduced by this distribution audit.

Completed public download and local installation results are recorded in the [0.10.4 verification report](release-verification-0.10.4.md).

Native portable testing exposed a taskbar registration boundary defect: Windows accepts a 255-unit relaunch command but rejects 256–259 UTF-16 units; the previous shortcut fallback started at 260 Python characters. Native regression tests reproduced the failure at 256 and 258 units, including an emoji path. The fallback now starts above 255 UTF-16 units and preserves the exact executable and settings path.

## Local verification — 2026-10-18

- Full regression: **272 passed, 7 skipped** (the seven tests require native Windows handles). Separate native Windows/task-progress run: **11 passed**.
- Native bootstrap: **14 fixture scenarios plus real install/start/update/uninstall passed**. Settings, user files, Start Menu, taskbar identity, and the uninstaller were checked.
- Fresh portable ZIP: direct launch with isolated settings and language runtimes removed from PATH passed; permanent installation, shortcut, startup, and service registrations were unchanged.
- Privacy review: 370 existing Git blobs and 146 working-tree files were scanned for common credential/private-key patterns without printing values; no matches. Payload paths and complete file hashes passed the distribution validator. The displayed screenshot contains demonstration projects.
- README was rendered through GitHub's Markdown API and inspected in Aside at desktop and 390-pixel content widths. Both download choices, images, and the separate Contentrium homepage button were checked. Public release verification is performed by the publisher after upload; see the version's verification report for final public hashes and installation evidence.

The updated Contentrium README template was applied during the audit: centered cover/title/badges/links, quick installation, a three-column feature table, a real application image with expandable examples, distribution/use/limits sections, and the brand footer. The repository has no license file, so the README states that fact instead of inventing a license badge or linking a missing file.

The first CI attempt passed regression, packaging, and native installer/update checks, then stopped before publication because its old WScript shortcut reader could not read Korean filenames on English Windows. An emoji filename reproduced the same empty-target failure locally. The native shortcut verifier now uses IShellLinkW to read the exact target, working directory, and arguments for both long-path and emoji cases; all 11 native checks passed again.
