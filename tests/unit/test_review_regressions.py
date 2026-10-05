import json
import uuid
from dataclasses import replace
from pathlib import Path
import pytest
from project_manager.models import Project,Conversation,Snapshot
from project_manager.bundles import export_project,verify_bundle,write_json
from project_manager.cleanup import cleanup_export
from project_manager.operations import import_bundle,transfer_project,recover_operation
from project_manager.restore_proof import verify_restored_bundle
from project_manager.journal import Journal
from project_manager.codex.portability import prepare_rollout
from project_manager.files import digest


class Adapter:
    def __init__(self,home,projects=(),threads=()):
        self.home=home;home.mkdir(exist_ok=True);self.projects=list(projects);self.threads=list(threads);self.deleted=[]
    def ensure_write_allowed(self): pass
    def snapshot(self): return Snapshot(tuple(self.projects),tuple(self.threads),'revision')
    def update_project(self,p): self.projects=[p if x.id==p.id else x for x in self.projects]
    def create_project(self,name,roots,key):
        pid='restored';self.projects=[p for p in self.projects if p.id!=pid]+[Project(pid,name,roots)];return pid
    def relocate_conversation(self,tid,cwd,runtime): self.threads=[replace(t,cwd=cwd,runtime_roots=runtime) if t.id==tid else t for t in self.threads]
    def assign_conversation(self,tid,pid): self.threads=[replace(t,project_id=pid) if t.id==tid else t for t in self.threads]
    def delete_thread(self,tid): self.deleted.append(tid);self.threads=[t for t in self.threads if t.id!=tid]
    def delete_project(self,pid): self.projects=[p for p in self.projects if p.id!=pid]
    def sync_desktop_state(self,*args): pass
    def read_thread(self,tid,*args):
        t=next(t for t in self.threads if t.id==tid);return {'id':tid,'cwd':str(t.cwd),'projectId':t.project_id}
    def call(self,*args): return {}
    def import_conversation(self,raw,tid,cwd,runtime,pid):
        path=prepare_rollout(raw,self.home,cwd,runtime)
        self.threads.append(Conversation(tid,None,cwd,runtime,False,None,'1',False,'T',path))
        if getattr(self,'fail_registration',False):
            self.fail_registration=False;raise TimeoutError('registered, response lost')
        self.assign_conversation(tid,pid)


def fixture(tmp_path):
    root=tmp_path/'source';root.mkdir();(root/'file.txt').write_text('original')
    p=Project('source','Source',(root,));tid=str(uuid.uuid4());raw=tmp_path/'chat.jsonl'
    raw.write_text(json.dumps({'type':'session_meta','payload':{'id':tid,'cwd':str(root),'runtime_workspace_roots':[str(root)]}})+'\n')
    t=Conversation(tid,p.id,root,(root,),False,None,'1',False,'T',raw)
    a=Adapter(tmp_path/'home',(p,),(t,));bundle=tmp_path/'bundle';export_project(p,a.snapshot(),bundle)
    write_json(bundle/'verification.json',{'restore_verified':True,'manifest_digest':digest(bundle/'manifest.json'),'bundle_id':json.loads((bundle/'manifest.json').read_text())['bundle_id']})
    return a,p,t,bundle


@pytest.mark.parametrize('drift',['roots','membership','parent','cwd','runtime'])
def test_cleanup_blocks_changed_relationship_before_deleting(tmp_path,drift):
    a,p,t,b=fixture(tmp_path)
    if drift=='roots': a.projects=[replace(p,roots=(tmp_path/'other',))]
    else:
        changes={'membership':{'project_id':'other'},'parent':{'parent_id':'outside'},'cwd':{'cwd':tmp_path/'elsewhere'},'runtime':{'runtime_roots':(tmp_path/'elsewhere',)}}
        a.threads=[replace(t,**changes[drift])]
    with pytest.raises(ValueError): cleanup_export(p,b,a,Journal(tmp_path/'journal.sqlite'))
    assert not a.deleted and (p.roots[0]/'file.txt').exists()


def test_unlisted_payload_is_not_verified(tmp_path):
    _,_,_,b=fixture(tmp_path);(b/'files/root-01/extra.txt').write_text('unverified')
    assert not verify_bundle(b).ok


def test_cleanup_blocks_reassigned_descendant(tmp_path):
    a,p,t,b=fixture(tmp_path);child=replace(t,id=str(uuid.uuid4()),parent_id=t.id);a.threads.append(child)
    b2=tmp_path/'with-child';export_project(p,a.snapshot(),b2)
    m=json.loads((b2/'manifest.json').read_text());write_json(b2/'verification.json',{'restore_verified':True,'bundle_id':m['bundle_id'],'manifest_digest':digest(b2/'manifest.json')})
    a.assign_conversation(child.id,'another-project')
    with pytest.raises(ValueError): cleanup_export(p,b2,a,Journal(tmp_path/'j.sqlite'))
    assert not a.deleted


def test_cleanup_preserves_folder_added_after_backup(tmp_path):
    a,p,_,b=fixture(tmp_path);(p.roots[0]/'new-empty').mkdir()
    with pytest.raises(ValueError): cleanup_export(p,b,a,Journal(tmp_path/'j.sqlite'))
    assert (p.roots[0]/'new-empty').exists()


def test_connection_failure_restores_each_previous_membership(tmp_path):
    from project_manager.operations import change_connections
    a,p,t,_=fixture(tmp_path);extra=replace(t,id=str(uuid.uuid4()),project_id=None);a.threads.append(extra)
    original=a.assign_conversation;count=0
    def once(tid,pid):
        nonlocal count
        count+=1
        if count==2: raise TimeoutError('connection interrupted')
        original(tid,pid)
    a.assign_conversation=once
    result=change_connections(a,replace(p,name='Renamed'),{t.id:None,extra.id:p.id},Journal(tmp_path/'j.sqlite'))
    assert result.state=='needs_recovery'
    assert a.projects[0].name==p.name
    assert {x.id:x.project_id for x in a.threads}=={t.id:p.id,extra.id:None}


def test_resume_rejects_another_bundle(tmp_path):
    _,p,t,b=fixture(tmp_path);j=Journal(tmp_path/'j.sqlite')
    a=Adapter(tmp_path/'target2');a.fail_registration=True
    assert import_bundle(b,{'root-01':tmp_path/'dest2'},a,j).state=='needs_recovery'
    op=j.pending()[0]
    a.threads=[]
    other=tmp_path/'other-bundle';export_project(p,Snapshot((p,),(t,),'1'),other)
    with pytest.raises(ValueError): import_bundle(other,{'root-01':tmp_path/'dest2'},a,j,resume_id=op['id'])


def test_resume_reconciles_registered_unassigned_chat(tmp_path):
    _,_,_,b=fixture(tmp_path);a=Adapter(tmp_path/'target');a.fail_registration=True;j=Journal(tmp_path/'j.sqlite')
    assert import_bundle(b,{'root-01':tmp_path/'dest'},a,j).state=='needs_recovery'
    op=j.pending()[0]
    assert import_bundle(b,{'root-01':tmp_path/'dest'},a,j,resume_id=op['id']).state=='completed'
    assert len(a.threads)==1 and a.threads[0].project_id=='restored'


def test_merge_failure_restores_target_roots_and_unassigned_child(tmp_path):
    a,p,t,_=fixture(tmp_path);root=tmp_path/'B';root.mkdir();b=Project('B','B',(root,));a.projects.append(b)
    child=replace(t,id=str(uuid.uuid4()),project_id=None,parent_id=t.id);a.threads.append(child)
    assign=a.assign_conversation;calls=0
    def fail_once(tid,pid):
        nonlocal calls
        calls+=1
        if calls==2: raise TimeoutError('assignment failed')
        assign(tid,pid)
    a.assign_conversation=fail_once;j=Journal(tmp_path/'j.sqlite')
    result=transfer_project(a,p,{str(p.roots[0]):tmp_path/'new-A'},j,tmp_path/'recovery',target=b)
    assert result.state=='needs_recovery'
    assert next(x for x in a.projects if x.id=='B').roots==b.roots
    assert next(x for x in a.threads if x.id==child.id).project_id is None
    assert recover_operation(j.pending()[0]['id'],a,j).state=='completed'
    assert next(x for x in a.projects if x.id=='B').roots==b.roots
    assert next(x for x in a.threads if x.id==child.id).project_id is None


@pytest.mark.parametrize('drift',['roots','parent','cwd','runtime'])
def test_restore_proof_rejects_path_and_ancestry_drift(tmp_path,drift):
    _,_,_,b=fixture(tmp_path);a=Adapter(tmp_path/'target');j=Journal(tmp_path/'j.sqlite')
    assert import_bundle(b,{'root-01':tmp_path/'dest'},a,j).state=='completed'
    t=a.threads[0]
    with t.rollout.open('a') as f:
        for record in [
            {'type':'event_msg','payload':{'type':'task_started','turn_id':'done'}},
            {'type':'response_item','payload':{'type':'message','role':'user','content':[{'text':'test'}]}},
            {'type':'response_item','payload':{'type':'message','role':'assistant','content':[{'text':'ok'}]}},
            {'type':'event_msg','payload':{'type':'task_complete','turn_id':'done'}}]: f.write(json.dumps(record)+'\n')
    if drift=='roots': a.projects=[replace(a.projects[0],roots=(tmp_path/'other',))]
    else:
        changes={'parent':{'parent_id':'wrong'},'cwd':{'cwd':tmp_path/'wrong'},'runtime':{'runtime_roots':(tmp_path/'wrong',)}}
        a.threads=[replace(t,**changes[drift])]
    assert not verify_restored_bundle(b,a).ok
