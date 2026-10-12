# Portable workflow implementation plan

> Execute inline with the executing-plans and test-driven-development skills; request a fresh whole-change review before release.

**Goal:** Make transfer between computers the primary workflow, with understandable deletion and a real Windows installer.

**Architecture:** Reuse verified folder bundles and Codex adapter operations. Add bounded bundle discovery and an import dialog; extend import to an existing project without overwriting its files or conversations. Keep recovery deletion and add explicit permanent deletion.

**Tech stack:** Python, PySide6, existing Codex app-server adapter, PyInstaller, Inno Setup.

## User-facing design and constraints

- One contextual action row. Projects: 보내기, 이름 변경, 삭제, 더 보기. Advanced move, merge and connection controls remain in menus and drag/drop.
- 가져오기 accepts either a bundle folder or an ancestor containing multiple bundles. Show project names, conversation counts, sizes, destination paths, and conflicts before writing. Select multiple bundles and choose new projects or one existing project.
- Merge keeps the existing project name and files. Imported files use separate unique subfolders. Existing conversation IDs are never overwritten: skip duplicates explicitly or choose a different store; show the duplicate count before execution.
- 삭제 defaults to recoverable deletion at the automatic local recovery location, with an optional location change. 완전 삭제 creates no recovery copy and requires an explicit irreversible-choice acknowledgement. File deletion is separately selected, never implied.
- All destructive checks still use fresh catalog data, ownership checks, protected-path checks and verified file inventory. No real user project/chat is used for destructive testing.
- GitHub README begins with a direct latest Setup.exe link. Installer installs all dependencies, registers Start menu and Windows uninstall, and keeps user settings on uninstall. Updater ZIP remains stable and supported.
- Existing authorization covers design, implementation, publishing and local replacement. No further approval gate is needed. Actual account continuation, a second physical computer and mobile are separate from isolated fixture validation.

## Tasks and verification

### 1. Portable discovery and import
- [x] RED: discover nested bundles without descending into bundle payloads; reject incomplete bundles; cancel scanning; preserve existing names/files/chats on merged import; duplicate skip leaves existing records untouched; resume a partial merge into the same target.
- [x] Implement `migration.py`: discover bundles, generate safe unique destinations, run batch imports. Extend `operations.import_bundle(..., target=None, skip_existing=False)` and recovery payload handling.
- [x] GREEN: unit discovery and real isolated Codex import integration tests.

### 2. Explicit deletion modes
- [x] RED: permanent deletion removes chosen records/files without a recovery directory; preserves unrelated records; stale/shared roots abort; partial failures are recorded without offering undo.
- [x] Implement permanent deletion inventory checks and terminal history report. Add mode choice and automatic recovery location to DeleteDialog.
- [x] GREEN: deletion integration and dialog tests including no mandatory directory picker.

### 3. Simplified UI
- [x] RED: import parent-folder workflow previews and executes chosen new/merge plan; hidden controls cannot target stale selections; permanent confirmation is disabled until acknowledged.
- [x] Implement import dialog and migration entry points. Remove duplicated visible action rows, retain shortcuts and context menus. Update help and user guide.
- [x] GREEN: UI suite and rendered native screenshots at normal and minimum sizes.

### 4. Installation and release
- [x] Add Inno Setup installer and build helper. Test install/uninstall in a dedicated location with isolated data and verify shortcuts/uninstaller.
- [x] Fresh reviewer, full suite, fixture acceptance and packaging/update verification.
- [ ] Publish v0.10.0 with Setup.exe, updater ZIP, metadata and checksums. Verify downloaded artifacts, native update button, then install on this computer and verify running version/settings/Start menu.

## Review focus

Interrupted multi-import, duplicate conversation IDs, unrelated files in the chosen destination, missing external-drive folders, and permanent deletion of linked/shared/protected roots must produce explicit results without overwriting unrelated data or suggesting nonexistent undo.

## Execution ledger

- Baseline: clean 4d2a543 / v0.9.0. No production data mutations during tests.
- Ruling: retain verified bundle format v1, portable ZIP updater asset names and existing settings path; the new setup is an additional first-install artifact.
- Ruling: use an implementation branch in the existing checkout because release scripts and installed portable paths are rooted here; no checkout relocation.

- Fresh whole-change reviewer findings were reproduced and fixed: current/selected project mismatch; hidden chat-status filter; changed merge target; interrupted batch-import recovery visibility; prepared uninstaller retry.
- Additional real isolated server regressions cover desktop alias changes in multi-bundle merging and resume, plus pre-0.10 import idempotency after a lost project-create response. Durable project keys prevent accidental duplicate projects.
- Final application suite: 228 passed, 1 skipped (189.97s); the skipped Windows HWND test separately passed on the native Windows platform (0.46s).
- Fixture acceptance passed with codex-cli 0.162.0-alpha.17.2. Actual account continuation, a second physical computer and mobile remain external checks, not inferred from fixture turns.
- Native screenshots inspected for project list, new/merged import, recoverable/permanent deletion and minimum 1060x680 window: no action overflow.
- Packaged update verification passed: native startup, taskbar registration, retained settings/user files, uninstaller preservation, and post-health cleanup. Installer launch was 4.266s; total verification 15.328s under concurrent QA load.
- QA helper initially read UTF-8 update receipts using the Korean Windows default codec. The application update succeeded; the verifier was corrected to explicit UTF-8 and rerun from a fresh QA installation.
- Installer QA caught an existing taskbar defect: a 263-character relaunch command returned 0x8007007a and cleared the window identity. A 278-character native regression reproduced it. A ShellExecute-compatible shortcut now preserves the executable, working directory and exact state argument. Both native Windows tests pass; release binaries are rebuilt after this fix.
- Microsoft relaunch target reference: https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-relaunchcommand . The fix does not require 8.3 names, which are unavailable on the QA volume.
- Rebuilt final payload: full suite 228 passed, 2 native-only skipped (186.17s); native suite 2 passed (0.99s). Isolated setup, Start menu, native startup, in-app update, uninstall, retained settings and retained user files all passed in setup-verification-0.10.0-native.
