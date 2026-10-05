import uuid
from pathlib import Path
import pytest
from project_manager.codex.adapter import CodexAdapter
from project_manager.operations import transfer_project, import_bundle
from project_manager.bundles import export_project
from project_manager.journal import Journal
from test_codex_portability import write_rollout


def make_project(adapter,root,name):
    root.mkdir();(root/'same.txt').write_text(name)
    pid=adapter.create_project(name,(root,),uuid.uuid4().hex)
    raw=root.parent/(name+'.jsonl');tid=str(uuid.uuid4());write_rollout(raw,tid,root)
    adapter.import_conversation(raw,tid,root,(root,),pid)
    return next(p for p in adapter.snapshot().projects if p.id==pid),tid


def test_merge_keeps_b_content(tmp_path):
    a=CodexAdapter(tmp_path/'home',isolated=True)
    pa,tid=make_project(a,tmp_path/'A','A');pb,_=make_project(a,tmp_path/'B','B')
    j=Journal(tmp_path/'journal.sqlite')
    result=transfer_project(a,pa,{str(pa.roots[0]):tmp_path/'B'/'A'},j,tmp_path/'recovery',target=pb,clean=False)
    assert result.state=='completed'
    assert (tmp_path/'B'/'same.txt').read_text()=='B'
    assert (tmp_path/'B'/'A'/'same.txt').read_text()=='A'
    assert a.read_thread(tid)['projectId']==pb.id
    assert Path(a.read_thread(tid)['cwd'])==tmp_path/'B'/'A'


def test_import_into_new_home(tmp_path):
    a=CodexAdapter(tmp_path/'home-a',isolated=True);p,tid=make_project(a,tmp_path/'A','A')
    bundle=tmp_path/'bundle';export_project(p,a.snapshot(),bundle)
    b=CodexAdapter(tmp_path/'home-b',isolated=True)
    result=import_bundle(bundle,{'root-01':tmp_path/'new-user'/'Restored'},b,Journal(tmp_path/'j.sqlite'))
    assert result.state=='completed'
    assert Path(b.read_thread(tid)['cwd'])==tmp_path/'new-user'/'Restored'
    assert '테스트 A입니다' in str(b.read_thread(tid,True))


def test_move_twice_then_backup_and_import(tmp_path):
    a=CodexAdapter(tmp_path/'home',isolated=True);p,tid=make_project(a,tmp_path/'A','A');j=Journal(tmp_path/'j.sqlite')
    for name in ('B','C'):
        destination=tmp_path/name
        result=transfer_project(a,p,{str(p.roots[0]):destination},j,tmp_path/'recovery')
        assert result.state=='completed',result.errors
        s=a.snapshot();p=next(x for x in s.projects if x.id==p.id);t=next(x for x in s.conversations if x.id==tid)
        assert t.runtime_roots==(destination,)
    bundle=tmp_path/'backup';export_project(p,a.snapshot(),bundle)
    b=CodexAdapter(tmp_path/'target-home',isolated=True)
    assert import_bundle(bundle,{'root-01':tmp_path/'D'},b,Journal(tmp_path/'import.sqlite')).state=='completed'
    assert next(t for t in b.snapshot().conversations if t.id==tid).runtime_roots==(tmp_path/'D',)
