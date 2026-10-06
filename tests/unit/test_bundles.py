from pathlib import Path
import json
import pytest
from project_manager.bundles import export_project, verify_bundle, read_transcript
from project_manager.models import Project, Conversation, Snapshot


def sample(tmp_path):
    root=tmp_path/'files';root.mkdir();(root/'테스트.txt').write_text('hello')
    raw=tmp_path/'rollout.jsonl'
    raw.write_text(json.dumps({'type':'session_meta','payload':{'id':'00000000-0000-4000-8000-000000000001','cwd':str(root)}})+'\n'+json.dumps({'type':'response_item','payload':{'type':'message','role':'user','content':[{'text':'hello'}]}})+'\n')
    p=Project('p','P',(root,))
    t=Conversation('00000000-0000-4000-8000-000000000001','p',root,(),False,None,'1',False,'T',raw)
    return p,Snapshot((p,),(t,),'1')


def test_bundle_readable_without_source_home(tmp_path):
    p,s=sample(tmp_path);b=tmp_path/'bundle'
    export_project(p,s,b)
    s.conversations[0].rollout.unlink()
    assert verify_bundle(b).ok
    assert read_transcript(b,s.conversations[0].id)[0]['text']=='hello'


def test_tamper_detected(tmp_path):
    p,s=sample(tmp_path);b=tmp_path/'bundle';export_project(p,s,b)
    (b/'files/root-01/테스트.txt').write_text('tamper')
    assert not verify_bundle(b).ok


def test_manifest_path_escape_rejected(tmp_path):
    p,s=sample(tmp_path);b=tmp_path/'bundle';export_project(p,s,b)
    m=json.loads((b/'manifest.json').read_text(encoding='utf-8'));m['conversations'][0]['rollout']='../secret'
    (b/'manifest.json').write_text(json.dumps(m))
    assert not verify_bundle(b).ok


def test_target_inside_source_rejected(tmp_path):
    p,s=sample(tmp_path)
    with pytest.raises(ValueError): export_project(p,s,p.roots[0]/'backup')


def test_project_files_named_like_bundle_metadata_are_included(tmp_path):
    p,s=sample(tmp_path)
    for name in ('checksums.jsonl','verification.json'): (p.roots[0]/name).write_text('user data')
    b=tmp_path/'bundle';assert export_project(p,s,b).ok
    assert verify_bundle(b).ok
    assert (b/'files/root-01/verification.json').read_text()=='user data'


def test_folder_grouped_chat_is_backed_up_without_rewriting_original_membership(tmp_path):
    from dataclasses import replace
    p,s=sample(tmp_path);t=replace(s.conversations[0],project_id=None)
    s=replace(s,conversations=(t,));b=tmp_path/'bundle'
    assert export_project(p,s,b,folder_memberships={t.id:p.id}).ok
    m=json.loads((b/'manifest.json').read_text(encoding='utf-8'))
    assert m['conversations'][0]['project_id'] is None and m['folder_grouped_threads']==[t.id]
    assert not m['dependencies'] and verify_bundle(b).ok and t.project_id is None
