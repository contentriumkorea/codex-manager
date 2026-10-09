import pytest
from project_manager.codex.adapter import CodexAdapter
from project_manager.journal import Journal
from project_manager.ui.batch import plan_projects,run_projects
from test_operations import make_project


def test_batch_merge_preserves_target_and_each_source(tmp_path):
    a=CodexAdapter(tmp_path/'home',isolated=True)
    pa,ta=make_project(a,tmp_path/'A','A');pb,tb=make_project(a,tmp_path/'B','B');pc,tc=make_project(a,tmp_path/'C','C')
    plans=plan_projects((pa,pb),pc.roots[0])
    result=run_projects(a,plans,pc,False,Journal(tmp_path/'journal.sqlite'),{},lambda *args:None,lambda:False)
    assert result.state=='completed',result.errors
    snap=a.snapshot();assert [p.id for p in snap.projects]==[pc.id]
    assert (pc.roots[0]/'A'/'same.txt').read_text()=='A'
    assert (pc.roots[0]/'B'/'same.txt').read_text()=='B'
    assert all(t.project_id==pc.id for t in snap.conversations)


def test_batch_destinations_distinct_and_cancel_preserves_sources(tmp_path):
    a=CodexAdapter(tmp_path/'home',isolated=True)
    pa,_=make_project(a,tmp_path/'A','same');pb,_=make_project(a,tmp_path/'B','same')
    dest=tmp_path/'dest';dest.mkdir();plans=plan_projects((pa,pb),dest)
    assert len({str(next(iter(mapping.values()))) for p,mapping in plans})==2
    result=run_projects(a,plans,None,False,Journal(tmp_path/'journal.sqlite'),{},lambda *args:None,lambda:True)
    assert result.state!='completed' and len(a.snapshot().projects)==2
    assert not list(dest.iterdir())
