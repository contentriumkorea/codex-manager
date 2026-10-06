import json,sqlite3
from project_manager.stores import discover_stores


def make(home):
    home.mkdir(parents=True)
    c=sqlite3.connect(home/'state_5.sqlite');c.executescript('CREATE TABLE projects(id,name); CREATE TABLE project_roots(project_id,position,path); CREATE TABLE threads(id,title,cwd,project_id,rollout_path,archived,updated_at);');c.close()
    (home/'.codex-global-state.json').write_text('{}')


def test_two_homes_are_discovered_and_canonical_duplicates_removed(tmp_path):
    a=tmp_path/'current';b=tmp_path/'old'/'custom-home';make(a);make(b)
    skills=tmp_path/'repo'/'.codex';skills.mkdir(parents=True);(skills/'config.toml').write_text('')
    result=discover_stores((a,a,tmp_path/'offline'),(tmp_path,),lambda:False)
    paths={s.home for s in result.stores}
    assert paths=={a.resolve(),b.resolve(),(tmp_path/'offline').resolve()}
    assert next(s for s in result.stores if s.home==b).available
    assert not next(s for s in result.stores if s.home.name=='offline').available


def test_generated_test_homes_are_excluded_and_scan_is_bounded(tmp_path):
    make(tmp_path/'.test-artifacts'/'fake');make(tmp_path/'real')
    result=discover_stores((),(tmp_path,),lambda:False,limit=20)
    assert [s.home for s in result.stores]==[tmp_path/'real']
    assert result.visited<=20
