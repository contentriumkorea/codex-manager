import json
import uuid
from pathlib import Path
import pytest


def manifest(path, name='Project'):
    path.mkdir(parents=True)
    data={'format_version':1,'bundle_id':str(uuid.uuid4()),'state':'complete',
          'project':{'id':'project','name':name,'roots':['D:/A']},'roots':[{'id':'root-01','original_path':'D:/A','bundle_path':'files/root-01'}],
          'conversations':[],'dependencies':[],'directories':[],'inventory':[]}
    (path/'manifest.json').write_text(json.dumps(data),encoding='utf-8')
    return data


def test_discovery_stops_at_bundle_and_reports_incomplete(tmp_path):
    from project_manager.migration import discover_bundles
    manifest(tmp_path/'outer'/'bundle');manifest(tmp_path/'outer'/'bundle'/'files'/'nested')
    broken=tmp_path/'unfinished';manifest(broken)
    (broken/'manifest.json').write_text('{')
    result=discover_bundles(tmp_path)
    assert [b.path for b in result.bundles]==[tmp_path/'outer'/'bundle']
    assert len(result.errors)==1 and 'unfinished' in result.errors[0]


def test_discovery_cancel_and_unrelated_manifests_do_not_hide_backups(tmp_path):
    from project_manager.migration import discover_bundles
    (tmp_path/'manifest.json').write_text('{"name":"web app"}')
    manifest(tmp_path/'nested')
    assert len(discover_bundles(tmp_path).bundles)==1
    with pytest.raises(ValueError,match='취소'):
        discover_bundles(tmp_path,cancelled=lambda:True)


def test_destination_collision_gets_unique_folder_and_duplicate_counts(tmp_path):
    from project_manager.migration import discover_bundles,plan_import
    from project_manager.models import Snapshot
    manifest(tmp_path/'backup',name='../CON:project')
    bundles=discover_bundles(tmp_path/'backup').bundles
    plans=plan_import(bundles,Snapshot((),(),''),tmp_path/'destination')
    path=plans[0].destinations['root-01'];path.mkdir(parents=True);(path/'keep.txt').write_text('keep')
    new=plan_import(bundles,Snapshot((),(),''),tmp_path/'destination')[0].destinations['root-01']
    assert new!=path and new.is_relative_to(tmp_path/'destination')
    assert (path/'keep.txt').read_text()=='keep'
