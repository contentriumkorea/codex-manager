import uuid
from pathlib import Path
from project_manager.codex.adapter import CodexAdapter
from test_codex_portability import write_rollout


def test_relocate_persists_new_path(tmp_path):
    home=tmp_path/'home';a=tmp_path/'a';a.mkdir();b=tmp_path/'b';b.mkdir()
    adapter=CodexAdapter(home,isolated=True)
    raw=tmp_path/'raw.jsonl';tid=str(uuid.uuid4());write_rollout(raw,tid,a)
    pid=adapter.create_project('A',(a,),'create-a')
    adapter.import_conversation(raw,tid,a,(a,),pid)
    adapter.relocate_conversation(tid,b,(b,))
    adapter.assign_conversation(tid,pid)
    assert Path(adapter.read_thread(tid)['cwd'])==b
    assert '테스트 A입니다' in str(adapter.read_thread(tid,True))
