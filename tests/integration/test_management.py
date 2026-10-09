import uuid
from dataclasses import replace
import pytest
from project_manager.codex.adapter import CodexAdapter
from project_manager.journal import Journal
from project_manager import management
from test_operations import make_project


def setup(tmp_path):
    adapter=CodexAdapter(tmp_path/'home',isolated=True)
    p,tid=make_project(adapter,tmp_path/'A','A')
    return adapter,p,tid,Journal(tmp_path/'state'/'journal.sqlite')


def test_rename_and_move_preserve_history_and_files(tmp_path):
    a,p,tid,j=setup(tmp_path);q,_=make_project(a,tmp_path/'B','B')
    assert management.rename_project(a,p,'새 프로젝트',j).state=='completed'
    assert next(x for x in a.snapshot().projects if x.id==p.id).name=='새 프로젝트'
    t=next(t for t in a.snapshot().conversations if t.id==tid)
    assert management.rename_thread(a,t,'새 대화 제목',j).state=='completed'
    t=next(t for t in a.snapshot().conversations if t.id==tid)
    assert t.title=='새 대화 제목'
    assert management.move_threads(a,(t,),q.id,j).state=='completed'
    current=next(t for t in a.snapshot().conversations if t.id==tid)
    assert current.project_id==q.id and current.cwd==p.roots[0]
    assert '테스트 A입니다' in str(a.read_thread(tid,True))
    assert (p.roots[0]/'same.txt').read_text()=='A'


@pytest.mark.parametrize('files',[False,True])
def test_delete_project_restores_from_verified_copy(tmp_path,files):
    a,p,tid,j=setup(tmp_path);q,other=make_project(a,tmp_path/'B','B')
    management.rename_thread(a,next(t for t in a.snapshot().conversations if t.id==tid),'보존할 제목',j)
    plan=management.plan_delete(a.snapshot(),project_id=p.id)
    result=management.delete_items(a,plan,j,tmp_path/'recovery',delete_files=files)
    assert result.state=='completed',result.errors
    assert not any(t.id==tid for t in a.snapshot().conversations)
    assert not any(x.id==p.id for x in a.snapshot().projects)
    assert (p.roots[0]/'same.txt').exists()==(not files)
    assert a.read_thread(other)['projectId']==q.id
    saved=[op for op in j.restorable() if op['payload']['kind']=='management-delete'];assert len(saved)==1
    result=management.recover_management(saved[0],a,j)
    assert result.state=='completed',result.errors
    restored=next(t for t in a.snapshot().conversations if t.id==tid)
    assert restored.title=='보존할 제목'
    assert '테스트 A입니다' in str(a.read_thread(tid,True))
    assert (p.roots[0]/'same.txt').read_text()=='A'
    assert not any(op['payload']['kind']=='management-delete' for op in j.restorable()) and not j.pending()


def test_delete_thread_keeps_project_and_rejects_stale_confirmation(tmp_path):
    a,p,tid,j=setup(tmp_path)
    plan=management.plan_delete(a.snapshot(),thread_ids=(tid,))
    a.call('thread/name/set',{'threadId':tid,'name':'다른 제목'})
    with pytest.raises(ValueError,match='바뀌'):
        management.delete_items(a,plan,j,tmp_path/'recovery')
    plan=management.plan_delete(a.snapshot(),thread_ids=(tid,))
    assert management.delete_items(a,plan,j,tmp_path/'recovery').state=='completed'
    assert a.project_exists(p.id) and (p.roots[0]/'same.txt').exists()


def test_cancel_and_shared_folder_never_delete(tmp_path):
    a,p,tid,j=setup(tmp_path)
    a.create_project('공유',p.roots,uuid.uuid4().hex)
    plan=management.plan_delete(a.snapshot(),project_id=p.id)
    with pytest.raises(ValueError,match='공유'):
        management.delete_items(a,plan,j,tmp_path/'recovery',delete_files=True)
    with pytest.raises(ValueError,match='취소'):
        management.delete_items(a,plan,j,tmp_path/'recovery',cancelled=lambda:True)
    assert a.project_exists(p.id) and a.read_thread(tid)


def test_preview_includes_children_and_blocks_running(tmp_path):
    from project_manager.models import Snapshot
    a,p,tid,j=setup(tmp_path);s=a.snapshot();t=s.conversations[0]
    child=replace(t,id=str(uuid.uuid4()),parent_id=t.id,project_id=None)
    plan=management.plan_delete(replace(s,conversations=(t,child)),thread_ids=(tid,))
    assert {x.id for x in plan.threads}=={t.id,child.id}
    with pytest.raises(ValueError,match='실행 중'):
        management.plan_delete(replace(s,conversations=(replace(t,running=True),)),thread_ids=(tid,))


def test_partial_delete_recovers_and_retains_unrelated_changes(tmp_path,monkeypatch):
    a,p,tid,j=setup(tmp_path)
    plan=management.plan_delete(a.snapshot(),project_id=p.id)
    original=a.delete_project
    monkeypatch.setattr(a,'delete_project',lambda pid:(_ for _ in ()).throw(RuntimeError('injected failure')))
    result=management.delete_items(a,plan,j,tmp_path/'recovery')
    assert result.state=='needs_recovery' and not a.snapshot().conversations
    (p.roots[0]/'new.txt').write_text('after deletion')
    monkeypatch.setattr(a,'delete_project',original)
    assert management.recover_management(j.pending()[0],a,j).state=='completed'
    assert a.read_thread(tid)['projectId']==p.id
    assert (p.roots[0]/'new.txt').read_text()=='after deletion'


def test_recovery_refuses_modified_backup_and_changed_files(tmp_path):
    a,p,tid,j=setup(tmp_path)
    plan=management.plan_delete(a.snapshot(),project_id=p.id)
    assert management.delete_items(a,plan,j,tmp_path/'recovery',delete_files=True).state=='completed'
    op=j.restorable()[0];backup=__import__('pathlib').Path(op['payload']['recovery'])
    original=(backup/(tid+'.jsonl')).read_bytes()
    (backup/(tid+'.jsonl')).write_bytes(b'changed')
    with pytest.raises(ValueError,match='검증'):
        management.recover_management(op,a,j)
    (backup/(tid+'.jsonl')).write_bytes(original)
    p.roots[0].mkdir();(p.roots[0]/'same.txt').write_text('new work')
    with pytest.raises(ValueError):management.recover_management(op,a,j)
    assert (p.roots[0]/'same.txt').read_text()=='new work'


def test_recovery_can_resume_when_title_update_failed(tmp_path,monkeypatch):
    a,p,tid,j=setup(tmp_path)
    t=next(t for t in a.snapshot().conversations if t.id==tid)
    management.rename_thread(a,t,'원래 제목',j)
    plan=management.plan_delete(a.snapshot(),project_id=p.id)
    assert management.delete_items(a,plan,j,tmp_path/'recovery').state=='completed'
    op=j.restorable()[0];original=management.set_thread_name
    monkeypatch.setattr(management,'set_thread_name',lambda *args:(_ for _ in ()).throw(RuntimeError('injected title failure')))
    with pytest.raises(RuntimeError):management.recover_management(op,a,j)
    monkeypatch.setattr(management,'set_thread_name',original)
    assert management.recover_management(op,a,j).state=='completed'
    assert next(t for t in a.snapshot().conversations if t.id==tid).title=='원래 제목'


def test_lightweight_ui_snapshot_can_rename_move_and_delete(tmp_path):
    a,p,tid,j=setup(tmp_path);q,_=make_project(a,tmp_path/'B','B')
    t=next(t for t in a.snapshot(include_runtime=False).conversations if t.id==tid)
    assert management.rename_thread(a,t,'UI 제목',j).state=='completed'
    t=next(t for t in a.snapshot(include_runtime=False).conversations if t.id==tid)
    assert management.move_threads(a,(t,),q.id,j).state=='completed'
    plan=management.plan_delete(a.snapshot(include_runtime=False),thread_ids=(tid,))
    assert management.delete_items(a,plan,j,tmp_path/'recovery').state=='completed'
    assert management.recover_management(j.restorable()[0],a,j).state=='completed'
    assert next(t for t in a.snapshot().conversations if t.id==tid).runtime_roots==p.roots


def test_recovery_ignores_legacy_ghost_project(tmp_path,monkeypatch):
    a,p,tid,j=setup(tmp_path);snapshot=a.snapshot
    sync=a.sync_desktop_state
    def fail_sync(*args,**kwargs):raise RuntimeError('desktop state failure')
    monkeypatch.setattr(a,'sync_desktop_state',fail_sync)
    plan=management.plan_delete(a.snapshot(),project_id=p.id)
    assert management.delete_items(a,plan,j,tmp_path/'recovery').state=='needs_recovery'
    def ghost(*args,**kwargs):
        s=snapshot(*args,**kwargs)
        return replace(s,projects=s.projects+(p,)) if not any(x.id==p.id for x in s.projects) else s
    monkeypatch.setattr(a,'snapshot',ghost);monkeypatch.setattr(a,'sync_desktop_state',sync)
    assert management.recover_management(j.pending()[0],a,j).state=='completed'
    assert a.project_exists(a.read_thread(tid)['projectId'])


def test_recovery_refuses_edited_replacement_project(tmp_path,monkeypatch):
    a,p,tid,j=setup(tmp_path);plan=management.plan_delete(a.snapshot(),project_id=p.id)
    management.delete_items(a,plan,j,tmp_path/'recovery');op=j.restorable()[0]
    original=a.import_conversation
    monkeypatch.setattr(a,'import_conversation',lambda *args:(_ for _ in ()).throw(RuntimeError('import failed')))
    with pytest.raises(RuntimeError):management.recover_management(op,a,j)
    replacement=a.snapshot().projects[0];changed=replace(replacement,name='사용자 변경')
    a.update_project(changed);monkeypatch.setattr(a,'import_conversation',original)
    with pytest.raises(ValueError,match='변경'):management.recover_management(op,a,j)
    assert a.snapshot().projects[0].name=='사용자 변경' and not a.snapshot().conversations


def test_retry_uses_authoritative_membership_despite_stale_desktop_assignment(tmp_path,monkeypatch):
    from project_manager.bundles import write_json
    from project_manager.codex.portability import prepare_rollout
    a,p,tid,j=setup(tmp_path);plan=management.plan_delete(a.snapshot(),project_id=p.id)
    management.delete_items(a,plan,j,tmp_path/'recovery');op=j.restorable()[0]
    write_json(a.home/'.codex-global-state.json',{'local-projects':{p.id:{'name':p.name,'rootPaths':[str(x) for x in p.roots]}},'thread-project-assignments':{tid:{'projectKind':'local','projectId':p.id}}})
    original=a.import_conversation
    def partial(raw,tid,cwd,runtime,target):
        saved=prepare_rollout(raw,a.home,cwd,runtime)
        a.call('thread/resume',{'threadId':tid,'path':str(saved),'cwd':str(cwd),'runtimeWorkspaceRoots':[str(x) for x in runtime]})
        raise RuntimeError('connection lost')
    monkeypatch.setattr(a,'import_conversation',partial)
    with pytest.raises(RuntimeError):management.recover_management(op,a,j)
    monkeypatch.setattr(a,'import_conversation',original)
    assert management.recover_management(op,a,j).state=='completed'
    assert a.read_thread(tid)['projectId']!=p.id


def test_batch_rename_and_undo_preserve_records(tmp_path):
    a,p,tid,j=setup(tmp_path);q,other=make_project(a,tmp_path/'B','B')
    before={t.id:t for t in a.snapshot().conversations}
    ts=tuple(before.values());names=tuple('Renamed '+str(i) for i in range(len(ts)))
    result=management.rename_threads(a,ts,names,j)
    assert result.state=='completed'
    op=next(x for x in j.restorable() if x['payload']['kind']=='rename-threads')
    assert management.recover_management(op,a,j).state=='completed'
    assert {t.id:t.title for t in a.snapshot().conversations}=={tid:t.title for tid,t in before.items()}
    assert not j.restorable()


def test_batch_validates_all_names_before_writing(tmp_path):
    a,p,tid,j=setup(tmp_path);q,other=make_project(a,tmp_path/'B','B')
    ts=a.snapshot().conversations
    with pytest.raises(ValueError):management.rename_threads(a,ts,('Valid',''),j)
    assert a.snapshot().conversations==ts and not j.pending()


def test_batch_partial_failure_can_restore_and_rejects_later_edits(tmp_path,monkeypatch):
    a,p,tid,j=setup(tmp_path);q,other=make_project(a,tmp_path/'B','B')
    ts=a.snapshot().conversations;original=management.set_thread_name
    calls=[]
    def fail_second(adapter,tid,name):
        calls.append(tid)
        if len(calls)==2:raise RuntimeError('simulated interruption')
        return original(adapter,tid,name)
    monkeypatch.setattr(management,'set_thread_name',fail_second)
    assert management.rename_threads(a,ts,('Changed A','Changed B'),j).state=='needs_recovery'
    op=j.pending()[0];monkeypatch.setattr(management,'set_thread_name',original)
    original(a,ts[1].id,'Later edit')
    with pytest.raises(ValueError):management.recover_management(op,a,j)
    assert next(t.title for t in a.snapshot().conversations if t.id==ts[0].id)=='Changed A'
    original(a,ts[1].id,ts[1].title)
    assert management.recover_management(op,a,j).state=='completed'
    assert {t.id:t.title for t in a.snapshot().conversations}=={t.id:t.title for t in ts}
