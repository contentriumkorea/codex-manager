import json
import shutil
import uuid
from pathlib import Path
from project_manager.codex.rpc import isolated_client
from project_manager.codex.portability import prepare_rollout


def write_rollout(path, thread_id, cwd):
    turn_id=str(uuid.uuid4())
    records = [
        {'timestamp':'2026-10-06T00:00:00Z','type':'session_meta','payload':{
            'id':thread_id,'timestamp':'2026-10-06T00:00:00Z','cwd':str(cwd),
            'originator':'codex_cli_rs','cli_version':'0.0.0','source':'cli','model_provider':'openai'}},
        {'timestamp':'2026-10-06T00:00:01Z','type':'event_msg','payload':{'type':'task_started','turn_id':turn_id}},
        {'timestamp':'2026-10-06T00:00:01Z','type':'event_msg','payload':{'type':'user_message','message':'테스트 A입니다','images':[],'local_images':[],'text_elements':[]}},
        {'timestamp':'2026-10-06T00:00:01Z','type':'response_item','payload':{
            'type':'message','id':'user-'+uuid.uuid4().hex,'role':'user','internal_chat_message_metadata_passthrough':{'turn_id':turn_id},'content':[{'type':'input_text','text':'테스트 A입니다'}]}},
        {'timestamp':'2026-10-06T00:00:02Z','type':'response_item','payload':{
            'type':'message','id':'agent-'+uuid.uuid4().hex,'role':'assistant','internal_chat_message_metadata_passthrough':{'turn_id':turn_id},'content':[{'type':'output_text','text':'테스트를 확인했습니다.'}]}},
        {'timestamp':'2026-10-06T00:00:03Z','type':'event_msg','payload':{'type':'task_complete','turn_id':turn_id}},
    ]
    path.write_text('\n'.join(json.dumps(x,ensure_ascii=False) for x in records)+'\n',encoding='utf-8')


def append_continuation(path):
    records=[
        {'type':'event_msg','payload':{'type':'task_started','turn_id':'fixture-continuation'}},
        {'type':'response_item','payload':{'type':'message','role':'user','content':[{'type':'input_text','text':'계속 테스트'}]}},
        {'type':'response_item','payload':{'type':'message','role':'assistant','content':[{'type':'output_text','text':'확인'}]}},
        {'type':'event_msg','payload':{'type':'task_complete','turn_id':'fixture-continuation'}}]
    with path.open('a',encoding='utf-8') as f:
        for record in records: f.write(json.dumps(record,ensure_ascii=False)+'\n')


def test_cross_home_rollout_restore(tmp_path):
    a = tmp_path/'a'; a.mkdir()
    b = tmp_path/'different-user'/'b'; b.mkdir(parents=True)
    thread_id = str(uuid.uuid4())
    raw = tmp_path/'rollout.jsonl'
    write_rollout(raw,thread_id,a)
    imported=prepare_rollout(raw,tmp_path/'home-b',b,(b,))
    with isolated_client(tmp_path/'home-b') as c:
        project = c.call('project/create',{'idempotencyKey':uuid.uuid4().hex,'name':'restored','roots':[{'path':str(b)}]})['project']
        resumed = c.call('thread/resume',{'threadId':thread_id,'path':str(imported),'cwd':str(b),'runtimeWorkspaceRoots':[str(b)]})['thread']
        assert resumed['id']==thread_id
        c.call('thread/metadata/update',{'threadId':thread_id,'projectId':project['id']})
        read=c.call('thread/read',{'threadId':thread_id,'includeTurns':True})['thread']
        assert read['projectId']==project['id']
        assert Path(read['cwd'])==b
        assert '테스트 A입니다' in json.dumps(read,ensure_ascii=False)
    with isolated_client(tmp_path/'home-b') as c:
        read=c.call('thread/read',{'threadId':thread_id,'includeTurns':True})['thread']
        assert Path(read['cwd'])==b
        assert read['projectId']==project['id']


def test_project_delete_keeps_files_and_thread(tmp_path):
    root=tmp_path/'files'; root.mkdir();(root/'keep.txt').write_text('keep')
    with isolated_client(tmp_path/'home') as c:
        p=c.call('project/create',{'idempotencyKey':uuid.uuid4().hex,'name':'A','roots':[{'path':str(root)}]})['project']
        tid=str(uuid.uuid4());raw=tmp_path/'raw.jsonl';write_rollout(raw,tid,root)
        raw=prepare_rollout(raw,tmp_path/'home',root,(root,))
        c.call('thread/resume',{'threadId':tid,'path':str(raw),'cwd':str(root)})
        c.call('thread/metadata/update',{'threadId':tid,'projectId':p['id']})
        c.call('project/delete',{'projectId':p['id']})
        assert c.call('thread/read',{'threadId':tid})['thread']['projectId'] is None
        assert (root/'keep.txt').read_text()=='keep'
        c.call('thread/delete',{'threadId':tid})
        assert all(x['id']!=tid for x in c.call('thread/list',{})['data'])
