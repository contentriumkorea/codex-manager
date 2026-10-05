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
- Packaging and independent final review pending.
