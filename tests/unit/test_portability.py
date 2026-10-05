import json
from project_manager.codex.portability import prepare_rollout


def test_only_latest_active_context_is_rebased(tmp_path):
    raw=tmp_path/'raw.jsonl'
    records=[{'type':'session_meta','payload':{'id':'00000000-0000-4000-8000-000000000001','cwd':'old'}},
             {'type':'turn_context','payload':{'cwd':'old','model':'m','turn_id':'past'}},
             {'type':'response_item','payload':{'type':'message','role':'user','content':[{'text':'old is history'}]}},
             {'type':'turn_context','payload':{'cwd':'old','model':'m','turn_id':'last'}},
             {'type':'world_state','payload':{'full':True,'state':{'environments':{'environments':{'local':{'cwd':'old'}}}}}}]
    raw.write_text('\n'.join(json.dumps(r) for r in records)+'\n')
    new=prepare_rollout(raw,tmp_path/'home',tmp_path/'new',(tmp_path/'new',))
    parsed=[json.loads(l) for l in new.read_text().splitlines()]
    assert parsed[1]['payload']['cwd']=='old'
    assert parsed[2]['payload']['content'][0]['text']=='old is history'
    assert parsed[-2]['payload']['cwd']==str((tmp_path/'new').resolve())
    assert parsed[-1]['payload']['state']['environments']['environments']['local']['cwd']==str((tmp_path/'new').resolve())
