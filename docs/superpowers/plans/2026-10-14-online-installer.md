# Online Installer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish one lightweight installer that always downloads the latest application, separately from versioned app updates.

**Architecture:** An Inno Setup bootstrap consumes a small `install.ini` release manifest and the existing verified ZIP. A fixed installer release remains unchanged across app releases; release tooling and a Windows workflow publish versioned payloads independently.

**Tech Stack:** Python 3.10, PySide6, PyInstaller, Inno Setup 6.7.3, GitHub Releases/Actions.

**Spec:** `docs/superpowers/specs/2026-10-14-online-installer-design.md`

## Global Constraints

- Bootstrap size budget: 5 MiB.
- Preserve existing ZIP/update.json protocol, AppId, installation location and user data.
- Fixed production repository; no runtime override of channel or archive URL.
- No payload replacement until successful download/hash verification/extraction; stop if the installed app is running.
- Stable installer release must never become the latest application release.

## Review Focus

1. An old saved installer follows a newer app release without any installer change (Task 2 native two-version test).
2. Failed/truncated/tampered download preserves the old installation (Task 2 negative tests).
3. Metadata attempts a different origin/version/path; reject before executing or installing files (Tasks 1/2 validation).
4. Bootstrap registry version and uninstall behavior survive later in-app updates (Task 2 end-to-end test).
5. Publishing the installer does not break old app update checks or existing download entry points (Task 3 public checks).

### Task 1: Separate release metadata and installer build

**Files:** `tools/build_release.py`, `tools/build_bootstrap.py`, `packaging/bootstrap.iss`, `tests/unit/test_release_packaging.py`.

**Interfaces:** `build(dist, output)` continues producing ZIP/update.json, also emits `install.ini`; `build_bootstrap(compiler, output, test=False, channel_base=None)` compiles the stable bootstrap without an application dist input.

- [ ] Write and run failing tests for a payload-only release, matching INI/JSON archive metadata and rejecting production channel overrides.
- [ ] Add manifest generation and the standalone bootstrap builder, keep the previous full installer as an explicitly named offline/developer path.
- [ ] Compile the bootstrap and verify size and payload exclusion.

### Task 2: Native online installation

**Files:** `packaging/bootstrap.iss`, `tools/verify_bootstrap.py`, relevant installer tests.

**Interfaces:** bootstrap reads INI protocol 1, validates version/hash/sizes, downloads the pinned version ZIP, extracts it and installs its `CodexManager` subtree. QA-only compile constants select an isolated app ID and local server.

- [ ] Create native fixture-server tests; verify they fail before implementation.
- [ ] Implement download/extraction progress, target/running-file guards and actual app version registration.
- [ ] Run same bootstrap against two versions; test bad hash, unavailable payload/manifest, cancellation/error preservation, version, shortcuts, startup, updater compatibility and uninstallation.
- [ ] Run full regression suite and a read-only whole-change review; fix reproduced issues.

### Task 3: Public release and local installation

**Files:** `.github/workflows/release.yml`, release publishing/verification tools, README, user guide and release notes.

**Interfaces:** workflow publishes immutable `installer-v1` only when absent, and versioned app assets with the app release marked latest; README links directly to the fixed installer.

- [ ] Prepare fixed-link documentation and Windows release workflow with only contents-write permission and pinned build dependencies.
- [ ] Publish reviewed source and app/installer releases; verify public installer size/hash, ZIP/INI/JSON consistency and old-client update discovery.
- [ ] Exercise public installation in an isolated directory, then update the user's local app with retained settings and a verified rollback copy.
- [ ] Record exact test results, asset URLs and installed version; report any actual publication limitation.

## Execution ledger

- 2026-10-14: Baseline a2c8132 / application 0.10.2. Local branch `online-installer`; initial worktree clean. Native execution selected under existing blanket implementation approval; one independent review follows implementation.
- Ruling: Reuse current checkout on a dedicated branch because the active project is a nested repository and existing build/installation verification tools target this checkout. No other agent is implementing changes.
- Ruling: Publishing is already authorized by the user; do not reopen approval gates. GitHub app can read/administer the public repository; gh and Git credentials are stale. Use app-authorized publication where supported and request authentication only if necessary after preparing the concrete release.
