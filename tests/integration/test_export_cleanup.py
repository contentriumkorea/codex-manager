import json
from project_manager.bundles import export_project
from project_manager.operations import import_bundle
from project_manager.operations import recover_operation
from project_manager.bundles import write_json,load_manifest
from project_manager.files import digest
from project_manager.codex.portability import prepare_rollout
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


def test_cleanup_recovery_recreates_deleted_project_despite_stale_legacy_catalog(tmp_path):
    source=CodexAdapter(tmp_path/'source-home',isolated=True)
    p,tid=make_project(source,tmp_path/'A','A');bundle=tmp_path/'bundle';export_project(p,source.snapshot(),bundle)
    m=load_manifest(bundle);write_json(bundle/'verification.json',{'restore_verified':True,'bundle_id':m['bundle_id'],'manifest_digest':digest(bundle/'manifest.json')})
    write_json(source.home/'.codex-global-state.json',{'local-projects':{p.id:{'id':p.id,'name':p.name,'rootPaths':[str(p.roots[0])]}}})
    saved=source.sync_desktop_state
    def fail(*args,**kwargs): raise OSError('desktop state unavailable')
    source.sync_desktop_state=fail;journal=Journal(tmp_path/'cleanup.sqlite')
    assert cleanup_export(p,bundle,source,journal).state=='needs_recovery'
    assert not source.project_exists(p.id)
    assert any(x.id==p.id for x in source.snapshot().projects) # stale UI mapping
    source.sync_desktop_state=saved
    result=recover_operation(journal.pending()[0]['id'],source,journal)
    assert result.state=='completed',result.errors
    restored=next(t for t in source.snapshot().conversations if t.id==tid)
    assert restored.project_id!=p.id and source.project_exists(restored.project_id)
    assert (p.roots[0]/'same.txt').read_text()=='A'


def test_cleanup_recovery_response_loss_uses_native_membership_over_stale_assignment(tmp_path):
    source=CodexAdapter(tmp_path/'source-home',isolated=True)
    p,tid=make_project(source,tmp_path/'A','A');bundle=tmp_path/'bundle';export_project(p,source.snapshot(),bundle)
    m=load_manifest(bundle);write_json(bundle/'verification.json',{'restore_verified':True,'bundle_id':m['bundle_id'],'manifest_digest':digest(bundle/'manifest.json')})
    write_json(source.home/'.codex-global-state.json',{'local-projects':{p.id:{'id':p.id,'name':p.name,'rootPaths':[str(p.roots[0])]}},
        'thread-project-assignments':{tid:{'projectKind':'local','projectId':p.id}}})
    def fail(*args,**kwargs): raise OSError('desktop sync failed')
    sync=source.sync_desktop_state;source.sync_desktop_state=fail;journal=Journal(tmp_path/'cleanup.sqlite')
    assert cleanup_export(p,bundle,source,journal).state=='needs_recovery'
    source.sync_desktop_state=sync
    normal_import=source.import_conversation
    def interrupted_import(raw,thread,cwd,runtime,pid):
        prepared=prepare_rollout(raw,source.home,cwd,runtime)
        source.call('thread/resume',{'threadId':thread,'path':str(prepared),'cwd':str(cwd),'runtimeWorkspaceRoots':[str(x) for x in runtime]})
        raise TimeoutError('registered, response lost before assignment')
    source.import_conversation=interrupted_import;op=journal.pending()[0]['id']
    assert recover_operation(op,source,journal).state=='needs_recovery'
    assert source.read_thread(tid).get('projectId') is None
    assert next(t for t in source.snapshot().conversations if t.id==tid).project_id==p.id
    source.import_conversation=normal_import
    result=recover_operation(op,source,journal)
    assert result.state=='completed',result.errors
    assert source.read_thread(tid)['projectId']!=p.id
