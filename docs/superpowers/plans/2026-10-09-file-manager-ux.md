# File-manager UX implementation plan

Goal: Make Codex Manager usable as a dark project file manager: choose a project, browse files, inspect conversations, then move, merge or back up from the same context.

Architecture: Keep existing data/transfer/backup safeguards. Replace the screen composition with a column-based project browser, a native asynchronous file browser, and a conversation/details pane. Put operation prerequisites in the operation flow. The user explicitly requested design, implementation and publication together; execute within that authorization.

Tech stack: Existing Python/PySide6, no new runtime dependencies. Version 0.3.0.

- [x] Build explorer navigation and project columns. Test project/root switching, missing folders, nested navigation, and selection after sort/search.
- [x] Rebuild the main layout: branded sidebar, location breadcrumb, project actions, file/conversation tabs, empty states and contextual menus. Preserve update/store cancellation.
- [x] Simplify dialogs and workflow text. Pick folders with buttons; explain backup/restore/cleanup steps; continue move/merge only after successful explicit conversation connection. Test failures/cancellation cannot continue mutation.
- [x] Run full suite, isolated operation acceptance and independent review. Inspect rendered normal/small-window UI with Korean long paths and empty states.
- [x] Build 0.3.0, verify the packaged UI and updater, publish Contentrium release and verify public asset hashes/old-version update discovery. Update the local app if idle; preserve user settings and originals.

Constraints: Existing files/chats are not modified by UI QA. File browsing is read-only; opening a file uses its registered Windows application only on an explicit user action. Project transfer and source cleanup retain their current verification requirements. Source cleanup is never the default.

Release evidence: 130 tests passed; public v0.3.0 archive SHA-256 2646d65c189ad6bc6af5358d3ba565a4d6dffce9a9308c2ebb367365ca05ad0f. Public old-version update button installed the new version and reported healthy startup in 12.87 seconds on this PC. The existing local installation was updated and launched, preserving settings.
