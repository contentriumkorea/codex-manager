# Execution ledger — 2026-10-06-codex-project-manager

Spec and plan: ../docs/superpowers/specs and ../docs/superpowers/plans in parent workspace.
User authorized implementation after creating test a / test b on 2026-10-06.
Ruling: Implement sequentially here with one independent final review; user requested implementation without selecting delegation.
Ruling: New repository on implementation branch provides isolation; no existing main branch or files are changed.
Pre-flight: Task 1 adapter is consumed by 2/4/6/7/8. Task 3 inventory by 4/5/6/8. Task 5 journal and operation plans by 6/7/8/9. Signatures are shared through models; no conflict found.
Live test folders verified. Do not mutate them until independent fixture tests pass. Test A contains generated user assets; preserve originals.

2026-10-06 execution evidence:
- RPC, catalog, file copy/hash, portable bundle, journal, transfer/import, cleanup guard, dark GUI implemented; 29 tests passed before final review.
- Actual test b: backup integrity verified, 12 files and 3 conversation turns imported into isolated target home. Live originals unchanged.
- Ruling: Use the app-bundled CLI 0.160.0 instead of separately installed 0.149.0. Older executable loses active world-state paths; cost if wrong: reject a supported version rather than silently use stale paths.
- Ruling: Windows maintenance requires closing the desktop app before writes; daemon/proxy is Unix-only. Read-only catalog and backups remain available. Cost: one close/reopen per batch.
- Ruling: Native path changes use resume + settings/update and restart readback; no delete/re-register fallback. Actual new-version persistence verified. The older-version experiment was discarded.
- Ruling: Interfaces use concrete export_project/transfer_project/import_bundle functions rather than an unused generic plan executor. Per-operation journal still captures the previewed changes. Cost: a future operation needs its own validated handler.
- Ruling: File merge defaults to a distinct subfolder; no flat merge/rename conflict wizard in this build. Existing different-content files are never overwritten. Cost: users flattening trees must resolve layouts separately.
- Partial recovery and post-import proof implemented. Actual-account continuation and mobile/second-PC checks are external acceptance work, not completed claims.
- Final independent review completed: one Critical and six Important findings admitted into one fix pass.
- Final: fixed source deletion scope drift — cleanup roots/membership/parent/cwd/runtime and descendant regressions RED→GREEN.
- Final: fixed stale runtime roots — latest persisted thread_settings_applied catalog regression RED→GREEN; move twice then export/import tested against bundled server.
- Final: fixed recovery bundle identity — different bundle regression RED→GREEN; relocated bundle locator provided.
- Final: fixed registration response loss — unassigned registered thread reconciliation regression RED→GREEN; intention journaled before RPC.
- Final: fixed merge rollback — target roots and individual previous memberships regression RED→GREEN; explicit recovery also restores them.
- Final: fixed restore proof scope — root/cwd/runtime/parent drift regressions RED→GREEN; full appended user/assistant completed turn required.
- Final: fixed unhashed payload inclusion — extra file regression RED→GREEN; strict manifest/checksum consistency and copying of declared inventory only.
- Additional deletion guard and connection journaling regressions RED→GREEN. Full suite 47/47 passed, 33.79 s, zero skipped.
- Isolated fixture acceptance: A merged into B, verified backup, new-home import, simulated continuation proof, source chats/project/files removed; pass. This is not actual account continuation.
- Final: Ruling: actual account continuation / second physical PC / mobile were declined by reviewer — ship as test build and keep these acceptance gates pending; cost if wrong: cross-device issues are not caught by local fixture tests.
- Final: Ruling: physical disk disconnect / cloud placeholders / hostile junction swapping were declined by reviewer — guarded paths and deterministic fixtures tested, no hardware guarantee; cost if wrong: interrupted operations require recovery, racing filesystem changes can still disrupt a transfer.
- Final: Ruling: no remote repository or integration destination was requested — preserve implementation branch and source folder; cost if wrong: integration remains a separate step.
- Large-history display: split lightweight display catalog from full operation validation; 2446 chats displayed in 0.23 s. Reverse line reader avoids quadratic copying of large image records.
- Packaging startup regression found an incompatible ICU DLL collected from Poppler on PATH. Confirmed working Qt loads Windows System32 ICU; distribution excludes external ICU.
- Latest full suite: 49/49 passed, 36.82 s, zero skipped.
- Full release acceptance remains blocked on actual account / second-PC / mobile verification; local fixture pass is explicitly separate.
- Final packaged onedir executable launched successfully and rendered the actual A/B catalog; final screenshot checked. Excluded incompatible Poppler ICU was the startup fix; runtime source program behavior is unchanged.
- Actual original test B's 12 file hashes and original conversation hash still match its pre-import backup; A/B project roots and IDs remain at their original paths.
- Transfer recovery copies are outside the destination project so normal backups do not capture rollback history accidentally.

2026-10-06 Codex Manager v0.2.0 publishing:
- User authorized Contentrium GitHub publication and startup/in-app updates.
- Renamed display and packaged executable; retained existing application state path.
- Public GitHub stable releases, async startup/manual checks, notes, preference, verified staging, external installer and restart implemented.
- Independent review found and fixed Windows helper cwd lock, inherited PowerShell module incompatibility, and missing interruption recovery.
- Actual rollback test asserts the failed replacement was swapped before restoration. Persisted first-rename interruption recovery tested separately.
- Full suite: 66 passed, 38.71 s. Native packaged install completed with version/path/PID health handshake; only verified prior program files removed, unknown user file and settings retained.
- No live user project/chat was moved or removed for updater verification. Actual other-PC/mobile continuation remains pending.

2026-10-06 Codex Manager v0.2.1 audit and publishing:
- Stable update discovery now prepares and verifies files in the background. Ready-button clicks launch installation immediately; clicks during download install automatically when ready. Active project jobs complete before handoff.
- Qt download/install threads remain responsive, failures keep the dialog and app open, cancellation blocks premature retry, and closing waits for owned workers. A native helper-ready handshake prevents exiting before installer startup succeeds.
- Download interruption retries once; API throttling falls back to stable public release metadata. Validated per-install prepared caches are reused. Native helpers serialize per installation with a named mutex.
- Fixed malformed settings startup and retained invalid originals; unplugged backup locations are retained; same-state duplicate app launches are blocked.
- Fixed configured SQLite-home reads, failed RPC initialization process leaks, malformed bundle validation and user files sharing bundle metadata names.
- Partial export-cleanup can restore missing chats/files while retaining surviving chats and changed files. Authoritative paginated project/list and native thread/read handle stale desktop mappings and lost project/chat registration responses.
- Independent reviewer found no remaining blocking finding within the reviewed source and focused real-server/Qt test scope.
- Final full suite: 92 passed, 61.46 s, zero skipped. Complete isolated fixture acceptance passed again; actual user A/B folders and chats were not moved or deleted.
- Final packaged v0.2.1 updater passed two native swaps, readiness acknowledgments, version/path/PID startup health, settings retention and unknown-file preservation. The archived v0.2.0 production updater/helper also installed the new package successfully.
- A private frozen Qt driver exercised the production update button against the final ZIP: one click, 16 ms click return, 0.899 s to parent exit after helper readiness, 21.306 s to healthy replacement on this computer. Timing is measured environment behavior, not a universal duration guarantee.
- Actual account continuation, physical second-PC and mobile checks remain pending; fixture results do not certify those environments.

2026-10-06 Codex Manager v0.2.2:
- Removed the bright project-list focus border; verified the packaged dark UI and transparent sidebar labels in a rendered fixture screenshot.
- Automatically calculate all project folder sizes without hashing file contents or blocking the GUI. Default descending size order, alternative sort orders, selected-project retention, cancellation, and explicitly partial results are covered.
- Read-only folder/parent grouping corrected 2,003 previously unassigned display entries in the actual catalog. Source memberships were not changed. The remaining 63 comprise 61 unmatched old/outside paths and two explicitly projectless chats.
- Folder-grouped chats are included in read-only backups, preserving original membership in the manifest. Explicit connection is required before move/merge/cleanup. Fresh backend checks prevent newly inferred chats being silently left behind. Uncaptured external workspace dependencies block import before mutation.
- Added bounded read-only multi-home discovery, retained registered/offline homes, a store selector and additional search locations. Atomic background verification preserves the previous adapter and screen when a switch fails. A copied-home catalog uses only unique recorded ID mappings, never guessed paths.
- Actual default discovery visited 10,234 directories in 4.156 seconds and found one store. Two-store switching, generated-directory exclusion, cancellation, and failed-switch recovery were verified with isolated fixtures.
- Final full suite: 121 passed, 54.07 seconds. Independent final review found no remaining Critical/Important blocker in its reviewed scope; focused 24 tests passed.
- Final production package updater passed helper readiness, swap, version/path/PID startup health, settings retention and preservation of unknown user files.
- A private frozen driver exercised the production Qt install button against the final ZIP: one click, 15 ms click return, 0.854 seconds to parent exit and 10.772 seconds to healthy v0.2.2 on this computer. Public asset verification and post-publication acceptance are recorded separately in the release evidence.
- No original user project/chat was moved, linked or deleted for these checks. Actual account continuation, physical second-PC and mobile checks remain pending.

2026-10-08 Codex Manager v0.2.3 logo update:
- Applied the user-approved monochrome folder/conversation PNG without regenerating it; SHA-256 7e5d2e9edcc9496b1634628848b3d59f4a696b7512361f689b96f12df2644625.
- Generated a Windows ICO with seven PNG frames (16/24/32/48/64/128/256px), embedded it in the executable, and packaged PNG/ICO resources for Qt windows and setuptools distributions.
- Set a version-independent Windows AppUserModelID. Source operation/chat logic is unchanged.
- Final full suite: 121 passed in 55.91 seconds. Independent review found no Critical/Important blocker in the inspected icon/startup/packaging scope.
- Inspected the final PE: all seven embedded RT_ICON resources match the ICO frame payloads. Queried the actual native window icon through WM_GETICON and compared its opaque pixels with the approved 32px Qt icon: pass.
- Final package updater passed helper readiness, replacement, v0.2.3 version/path/PID health, existing settings retention and preservation of unknown user files. Public release and installed-user shortcut checks are recorded in local release evidence.
