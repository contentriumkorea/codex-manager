from pathlib import Path
import pytest
from project_manager.codex.adapter import CodexAdapter
from project_manager.bundles import export_project
from project_manager.operations import import_bundle, recover_operation
from project_manager.journal import Journal
from test_operations import make_project


def test_parent_discovery_and_merge_preserves_target(tmp_path):
    from project_manager.migration import discover_bundles, plan_import
    source = CodexAdapter(tmp_path/'source-home', isolated=True)
    a, tid = make_project(source, tmp_path/'A', 'A')
    bundle = tmp_path/'external'/'year'/'backup'; bundle.parent.mkdir(parents=True)
    export_project(a, source.snapshot(), bundle)
    found = discover_bundles(tmp_path/'external')
    assert [x.path for x in found.bundles] == [bundle]
    target = CodexAdapter(tmp_path/'target-home', isolated=True)
    b, old = make_project(target, tmp_path/'B', 'B')
    plans = plan_import(found.bundles, target.snapshot(), tmp_path/'destination', b.id)
    result = import_bundle(bundle, plans[0].destinations, target, Journal(tmp_path/'state'/'j.sqlite'), target=b)
    assert result.state == 'completed', result.errors
    current = target.snapshot()
    assert len(current.projects) == 1 and current.projects[0].name == 'B'
    assert (b.roots[0]/'same.txt').read_text() == 'B'
    assert target.read_thread(old)['projectId'] == b.id
    assert target.read_thread(tid)['projectId'] == b.id
    restored = Path(target.read_thread(tid)['cwd'])
    assert restored != b.roots[0] and (restored/'same.txt').read_text() == 'A'


def test_duplicate_skip_keeps_original_conversation_and_rejects_implicit_overwrite(tmp_path):
    a = CodexAdapter(tmp_path/'home', isolated=True); p, tid = make_project(a, tmp_path/'A', 'A')
    bundle=tmp_path/'backup'; export_project(p,a.snapshot(),bundle)
    q,_=make_project(a,tmp_path/'B','B'); j=Journal(tmp_path/'state'/'j.sqlite')
    with pytest.raises(ValueError, match='같은 ID'):
        import_bundle(bundle, {'root-01':tmp_path/'refused'},a,j,target=q)
    before=a.read_thread(tid,True)
    result=import_bundle(bundle,{'root-01':tmp_path/'copy'},a,j,target=q,skip_existing=True)
    assert result.state=='completed',result.errors
    assert a.read_thread(tid,True)==before
    assert (tmp_path/'copy'/'same.txt').read_text()=='A'
    assert '1' in result.codex_status


@pytest.mark.parametrize('after_sync',[False,True])
def test_partial_merge_resumes_same_target(tmp_path,monkeypatch,after_sync):
    source=CodexAdapter(tmp_path/'source',isolated=True);p,tid=make_project(source,tmp_path/'A','A')
    bundle=tmp_path/'backup';export_project(p,source.snapshot(),bundle)
    target=CodexAdapter(tmp_path/'target',isolated=True);q,old=make_project(target,tmp_path/'B','B')
    j=Journal(tmp_path/'state'/'j.sqlite');original=target.import_conversation
    monkeypatch.setattr(target,'import_conversation',lambda *args:(_ for _ in ()).throw(RuntimeError('test interruption')))
    if after_sync:
        from project_manager.bundles import write_json
        monkeypatch.setattr(target,'import_conversation',original)
        def interrupted_sync(*args):
            write_json(target.home/'.codex-global-state.json',{'app-server-project-id-by-legacy-project-id-by-host':{'local:'+str(target.home):{q.id:q.id}}})
            raise RuntimeError('test after desktop sync')
        monkeypatch.setattr(target,'sync_desktop_state',interrupted_sync)
    result=import_bundle(bundle,{'root-01':tmp_path/'imported'},target,j,target=q)
    assert result.state=='needs_recovery'
    monkeypatch.setattr(target,'import_conversation',original)
    if after_sync:monkeypatch.setattr(target,'sync_desktop_state',lambda *args:None)
    result=recover_operation(result.report_id,target,j)
    assert result.state=='completed',result.errors
    assert len(target.snapshot().projects)==1
    assert target.read_thread(old)['projectId']==q.id==target.read_thread(tid)['projectId']


def test_resumes_legacy_import_without_creating_second_project(tmp_path):
    from project_manager.bundles import load_manifest
    from project_manager.files import digest
    a=CodexAdapter(tmp_path/'source',isolated=True);p,tid=make_project(a,tmp_path/'A','A')
    bundle=tmp_path/'backup';export_project(p,a.snapshot(),bundle);m=load_manifest(bundle)
    b=CodexAdapter(tmp_path/'target',isolated=True);dest=tmp_path/'restored';dest.mkdir()
    pid=b.create_project('A',(dest,),'import-'+m['bundle_id'])
    j=Journal(tmp_path/'state'/'j.sqlite')
    j.begin('legacy-import',{'kind':'import','home':str(b.home),'resources':['bundle:'+m['bundle_id']],
       'bundle':str(bundle),'bundle_id':m['bundle_id'],'manifest_digest':digest(bundle/'manifest.json'),'destinations':{'root-01':str(dest)}})
    # Simulate the old create_project commit before its response reached the journal.
    assert recover_operation('legacy-import',b,j).state=='completed'
    assert [x.id for x in b.snapshot().projects]==[pid]
    assert b.read_thread(tid)['projectId']==pid


def test_batch_import_rejects_target_changed_after_preview(tmp_path):
    from dataclasses import replace
    from project_manager.migration import discover_bundles,plan_import,run_imports
    a=CodexAdapter(tmp_path/'source',isolated=True);p,tid=make_project(a,tmp_path/'A','A')
    bundle=tmp_path/'backup';export_project(p,a.snapshot(),bundle)
    b=CodexAdapter(tmp_path/'target',isolated=True);q,_=make_project(b,tmp_path/'B','B')
    plans=plan_import(discover_bundles(bundle).bundles,b.snapshot(),q.roots[0],q.id)
    new=tmp_path/'Changed';new.mkdir();b.update_project(replace(q,roots=(new,)))
    result=run_imports(plans,b,Journal(tmp_path/'state'/'j.sqlite'),False)
    assert result.state=='failed'
    assert not plans[0].destinations['root-01'].exists()
    assert all(t.id!=tid for t in b.snapshot().conversations)


def test_batch_import_recovery_is_available_on_parent_and_updates_status(tmp_path,monkeypatch):
    from project_manager.migration import discover_bundles,plan_import,run_imports
    from project_manager.ui.activity import related_pending
    a=CodexAdapter(tmp_path/'source',isolated=True);p,_=make_project(a,tmp_path/'A','A')
    bundle=tmp_path/'backup';export_project(p,a.snapshot(),bundle)
    b=CodexAdapter(tmp_path/'target',isolated=True);j=Journal(tmp_path/'state'/'j.sqlite')
    plans=plan_import(discover_bundles(bundle).bundles,b.snapshot(),tmp_path/'dest')
    original=b.sync_desktop_state
    monkeypatch.setattr(b,'sync_desktop_state',lambda *args:(_ for _ in ()).throw(RuntimeError('interrupted')))
    result=run_imports(plans,b,j,False)
    assert result.state=='needs_recovery'
    children=related_pending(j.operation(result.report_id),j)
    assert len(children)==1
    monkeypatch.setattr(b,'sync_desktop_state',original)
    assert recover_operation(children[0]['id'],b,j).state=='completed'
    parent=j.operation(result.report_id)
    assert parent['payload']['items'][0]['status']=='completed'
    assert parent['payload']['result_state']=='completed'


def test_multiple_bundles_merge_even_when_desktop_alias_is_added(tmp_path,monkeypatch):
    from project_manager.migration import discover_bundles,plan_import,run_imports
    from project_manager.bundles import write_json
    a=CodexAdapter(tmp_path/'source',isolated=True)
    exports=tmp_path/'exports';exports.mkdir()
    tids=[]
    for name in ('A','C'):
        p,tid=make_project(a,tmp_path/name,name);tids.append(tid);export_project(p,a.snapshot(),exports/name)
    b=CodexAdapter(tmp_path/'target',isolated=True);q,_=make_project(b,tmp_path/'B','B')
    plans=plan_import(discover_bundles(exports).bundles,b.snapshot(),q.roots[0],q.id)
    def sync(projects,*args):
        write_json(b.home/'.codex-global-state.json',{'app-server-project-id-by-legacy-project-id-by-host':{'local:'+str(b.home):{q.id:q.id}}})
    monkeypatch.setattr(b,'sync_desktop_state',sync)
    result=run_imports(plans,b,Journal(tmp_path/'state'/'j.sqlite'),False)
    assert result.state=='completed',result.errors
    assert all(b.read_thread(tid)['projectId']==q.id for tid in tids)
