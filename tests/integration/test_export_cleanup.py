import json
from project_manager.bundles import export_project
from project_manager.operations import import_bundle
from project_manager.restore_proof import verify_restored_bundle
from project_manager.cleanup import cleanup_export
from project_manager.codex.adapter import CodexAdapter
from project_manager.journal import Journal
from test_operations import make_project
from test_codex_portability import append_continuation


def test_export_restore_proof_then_cleanup(tmp_path):
    source=CodexAdapter(tmp_path/'source-home',isolated=True)
    p,tid=make_project(source,tmp_path/'A','A');bundle=tmp_path/'bundle'
    export_project(p,source.snapshot(),bundle)
    target=CodexAdapter(tmp_path/'target-home',isolated=True)
    assert import_bundle(bundle,{'root-01':tmp_path/'restored'},target,Journal(tmp_path/'import.sqlite')).state=='completed'
    assert not verify_restored_bundle(bundle,target).ok
    # Simulate a successful post-import turn only in the disposable fixture.
    t=next(t for t in target.snapshot().conversations if t.id==tid)
    append_continuation(t.rollout)
    assert verify_restored_bundle(bundle,target).ok
    result=cleanup_export(p,bundle,source,Journal(tmp_path/'cleanup.sqlite'))
    assert result.state=='completed'
    assert not p.roots[0].exists()
    assert not any(t.id==tid for t in source.snapshot().conversations)
    assert (tmp_path/'restored/same.txt').read_text()=='A'
