import json,uuid
from dataclasses import replace
import pytest
from test_review_regressions import fixture
from project_manager.bundles import export_project,load_manifest,write_json
from project_manager.cleanup import cleanup_export
from project_manager.operations import recover_operation
from project_manager.journal import Journal
from project_manager.files import digest


def interrupted(tmp_path):
    a,p,t,_=fixture(tmp_path);tid=str(uuid.uuid4());raw=tmp_path/'second.jsonl'
    raw.write_text(json.dumps({'type':'session_meta','payload':{'id':tid,'cwd':str(t.cwd),'runtime_workspace_roots':[str(t.cwd)]}})+'\n')
    a.threads.append(replace(t,id=tid,rollout=raw))
    bundle=tmp_path/'both';export_project(p,a.snapshot(),bundle);m=load_manifest(bundle)
    write_json(bundle/'verification.json',{'restore_verified':True,'bundle_id':m['bundle_id'],'manifest_digest':digest(bundle/'manifest.json')})
    original=a.delete_thread;calls=[]
    def fail(tid):
        calls.append(tid)
        if len(calls)==2: raise TimeoutError('second deletion failed')
        original(tid)
    a.delete_thread=fail;j=Journal(tmp_path/'j.sqlite')
    assert cleanup_export(p,bundle,a,j).state=='needs_recovery'
    return a,p,bundle,j,j.pending()[0]['id']


def test_partial_cleanup_restores_missing_chat_files_and_retains_survivor_append(tmp_path):
    a,p,b,j,op=interrupted(tmp_path);survivor=a.threads[0]
    with survivor.rollout.open('a') as f: f.write(json.dumps({'type':'response_item','payload':{'type':'message','role':'user','content':[{'text':'new message'}]}})+'\n')
    survivor_bytes=survivor.rollout.read_bytes();(p.roots[0]/'file.txt').unlink();(p.roots[0]/'new.txt').write_text('new data')
    assert recover_operation(op,a,j).state=='completed'
    assert len(a.threads)==2 and not j.pending()
    assert survivor.rollout.read_bytes()==survivor_bytes
    assert (p.roots[0]/'file.txt').read_text()=='original'
    assert (p.roots[0]/'new.txt').read_text()=='new data'
    assert recover_operation(op,a,j).state=='completed' and len(a.threads)==2


def test_partial_cleanup_registration_response_loss_can_resume(tmp_path):
    a,p,b,j,op=interrupted(tmp_path);a.fail_registration=True
    assert recover_operation(op,a,j).state=='needs_recovery'
    assert len(a.threads)==2
    assert recover_operation(op,a,j).state=='completed'
    assert len(a.threads)==2


def test_partial_cleanup_rejects_changed_surviving_history_before_mutation(tmp_path):
    a,p,b,j,op=interrupted(tmp_path);a.threads[0].rollout.write_text('divergent history')
    with pytest.raises(ValueError): recover_operation(op,a,j)
    assert len(a.threads)==1 and j.pending()


def test_partial_cleanup_accepts_same_relocated_backup(tmp_path):
    a,p,b,j,op=interrupted(tmp_path);moved=tmp_path/'relocated';assert b.parent==moved.parent==tmp_path
    b.rename(moved)
    assert recover_operation(op,a,j,bundle_override=moved).state=='completed'


def test_project_creation_response_loss_can_resume_without_shared_root_false_conflict(tmp_path):
    a,p,b,j,op=interrupted(tmp_path);a.projects=[];a.threads=[]
    normal=a.create_project;attempts=[]
    def create(*args):
        pid=normal(*args);attempts.append(pid)
        if len(attempts)==1: raise TimeoutError('project created, response lost')
        return pid
    a.create_project=create
    assert recover_operation(op,a,j).state=='needs_recovery'
    assert recover_operation(op,a,j).state=='completed'
    assert len(a.threads)==2 and not j.pending()
