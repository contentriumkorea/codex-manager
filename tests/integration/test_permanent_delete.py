import pytest
from project_manager.codex.adapter import CodexAdapter
from project_manager.management import plan_delete, delete_permanently
from project_manager.journal import Journal
from test_operations import make_project


@pytest.mark.parametrize('files',[False,True])
def test_permanent_delete_has_no_recovery_copy_and_preserves_other_project(tmp_path,files):
    a=CodexAdapter(tmp_path/'home',isolated=True);p,tid=make_project(a,tmp_path/'A','A');q,other=make_project(a,tmp_path/'B','B')
    j=Journal(tmp_path/'state'/'j.sqlite');plan=plan_delete(a.snapshot(),project_id=p.id)
    result=delete_permanently(a,plan,j,delete_files=files)
    assert result.state=='completed',result.errors
    assert not a.project_exists(p.id) and all(t.id!=tid for t in a.snapshot().conversations)
    assert (p.roots[0]/'same.txt').exists()==(not files)
    assert a.read_thread(other)['projectId']==q.id and (q.roots[0]/'same.txt').read_text()=='B'
    assert not j.restorable() and not j.pending()
    assert list((tmp_path/'state').iterdir())==[j.path]


def test_shared_folder_stale_plan_and_cancel_are_not_deleted(tmp_path):
    a=CodexAdapter(tmp_path/'home',isolated=True);p,tid=make_project(a,tmp_path/'A','A');j=Journal(tmp_path/'state'/'j.sqlite')
    plan=plan_delete(a.snapshot(),project_id=p.id)
    with pytest.raises(ValueError,match='취소'):delete_permanently(a,plan,j,cancelled=lambda:True)
    a.create_project('shared',p.roots,'shared')
    with pytest.raises(ValueError,match='공유'):delete_permanently(a,plan,j,delete_files=True)
    a.call('thread/name/set',{'threadId':tid,'name':'changed'})
    with pytest.raises(ValueError,match='바뀌'):delete_permanently(a,plan,j)
    assert a.project_exists(p.id) and (p.roots[0]/'same.txt').exists()


def test_partial_permanent_failure_does_not_offer_fake_recovery(tmp_path,monkeypatch):
    from project_manager.ui.activity import recovery_available,retry_candidates
    a=CodexAdapter(tmp_path/'home',isolated=True);p,tid=make_project(a,tmp_path/'A','A');j=Journal(tmp_path/'state'/'j.sqlite')
    monkeypatch.setattr(a,'delete_project',lambda _:(_ for _ in ()).throw(RuntimeError('delete interrupted')))
    result=delete_permanently(a,plan_delete(a.snapshot(),project_id=p.id),j)
    assert result.state=='failed' and result.errors
    op=j.operation(result.report_id)
    assert not recovery_available(op,j) and not retry_candidates(op,j) and not j.pending()
    assert a.project_exists(p.id) and all(t.id!=tid for t in a.snapshot().conversations)
