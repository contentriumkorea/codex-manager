import json
import sqlite3
from project_manager.catalog import read_catalog


def test_membership_uses_assignment_not_path(tmp_path):
    c=sqlite3.connect(tmp_path/'state_5.sqlite')
    c.executescript('CREATE TABLE projects(id,name); CREATE TABLE project_roots(project_id,position,path); CREATE TABLE threads(id,title,cwd,project_id,rollout_path,archived,updated_at,has_user_event);')
    c.execute('INSERT INTO projects VALUES (?,?)',('canonical','B'))
    c.execute('INSERT INTO project_roots VALUES (?,?,?)',('canonical',0,str(tmp_path/'b')))
    c.execute('INSERT INTO threads VALUES (?,?,?,?,?,?,?,?)',('t','title',str(tmp_path/'a'),None,'missing',0,1,1));c.commit();c.close()
    (tmp_path/'.codex-global-state.json').write_text(json.dumps({'local-projects':{'legacy':{'id':'legacy','name':'B','rootPaths':[str(tmp_path/'b')]}},'thread-project-assignments':{'t':{'projectId':'legacy','projectKind':'local'}},'app-server-project-id-by-legacy-project-id-by-host':{'local:'+str(tmp_path):{'legacy':'canonical'}}}))
    snapshot=read_catalog(tmp_path)
    assert snapshot.conversations[0].project_id=='canonical'
    assert len(snapshot.projects)==1


def test_no_path_guessing_when_unassigned(tmp_path):
    c=sqlite3.connect(tmp_path/'state_5.sqlite')
    c.executescript('CREATE TABLE projects(id,name); CREATE TABLE project_roots(project_id,position,path); CREATE TABLE threads(id,title,cwd,project_id,rollout_path,archived,updated_at,has_user_event);')
    c.execute('INSERT INTO projects VALUES (?,?)',('p','P'))
    c.execute('INSERT INTO project_roots VALUES (?,?,?)',('p',0,str(tmp_path)))
    c.execute('INSERT INTO threads VALUES (?,?,?,?,?,?,?,?)',('t','T',str(tmp_path),None,'missing',1,1,1));c.commit();c.close()
    assert read_catalog(tmp_path).conversations[0].project_id is None


def test_latest_thread_settings_roots_override_initial_metadata(tmp_path):
    c=sqlite3.connect(tmp_path/'state_5.sqlite')
    c.executescript('CREATE TABLE threads(id,title,cwd,project_id,rollout_path,archived,updated_at,has_user_event);')
    raw=tmp_path/'raw.jsonl';old=tmp_path/'old';new=tmp_path/'new'
    raw.write_text('\n'.join(json.dumps(r) for r in [
        {'type':'session_meta','payload':{'cwd':str(old),'runtime_workspace_roots':[str(old)]}},
        {'type':'event_msg','payload':{'type':'thread_settings_applied','thread_settings':{'cwd':str(new),'runtime_workspace_roots':[str(new)]}}}])+'\n')
    c.execute('INSERT INTO threads VALUES (?,?,?,?,?,?,?,?)',('t','T',str(new),'p',str(raw),0,1,1));c.commit();c.close()
    assert read_catalog(tmp_path).conversations[0].runtime_roots==(new,)


def test_reverse_reader_keeps_long_records_and_unicode_boundaries():
    from io import BytesIO
    from project_manager.catalog import reverse_lines
    lines=[json.dumps({'text':'한글'*1000000},ensure_ascii=False).encode(),'last'.encode()]
    data=b'\n'.join(lines)+b'\n'
    assert [x for x in reverse_lines(BytesIO(data),len(data)) if x]==list(reversed(lines))


def test_display_catalog_does_not_scan_raw_histories(tmp_path,monkeypatch):
    import project_manager.catalog as catalog
    c=sqlite3.connect(tmp_path/'state_5.sqlite')
    c.executescript('CREATE TABLE threads(id,title,cwd,project_id,rollout_path,archived,updated_at,has_user_event);')
    raw=tmp_path/'raw.jsonl';raw.write_text('not needed for display')
    c.execute('INSERT INTO threads VALUES (?,?,?,?,?,?,?,?)',('t','T',str(tmp_path),'p',str(raw),0,1,1));c.commit();c.close()
    def forbidden(*args): raise AssertionError('display scanned history')
    monkeypatch.setattr(catalog,'active_runtime_roots',forbidden)
    assert catalog.read_catalog(tmp_path,include_runtime=False).conversations[0].title=='T'


def test_configured_sqlite_home_matches_server_catalog(tmp_path):
    home=tmp_path/'home';home.mkdir();database=tmp_path/'database';database.mkdir()
    (home/'config.toml').write_text('sqlite_home = '+json.dumps(str(database))+'\n')
    c=sqlite3.connect(database/'state_5.sqlite')
    c.executescript('CREATE TABLE projects(id,name); CREATE TABLE project_roots(project_id,position,path); CREATE TABLE threads(id,title,cwd,project_id,rollout_path,archived,updated_at,has_user_event);')
    c.execute('INSERT INTO projects VALUES (?,?)',('p','P'));c.commit();c.close()
    assert read_catalog(home).projects[0].id=='p'


def test_copied_home_uses_recorded_old_home_mapping_without_duplicate_projects(tmp_path):
    c=sqlite3.connect(tmp_path/'state_5.sqlite')
    c.executescript('CREATE TABLE projects(id,name); CREATE TABLE project_roots(project_id,position,path); CREATE TABLE threads(id,title,cwd,project_id,rollout_path,archived,updated_at);')
    c.execute('INSERT INTO projects VALUES (?,?)',('canonical','P'));c.execute('INSERT INTO project_roots VALUES (?,?,?)',('canonical',0,str(tmp_path/'files')))
    c.execute('INSERT INTO threads VALUES (?,?,?,?,?,?,?)',('t','T',str(tmp_path/'files'),None,'missing',0,1));c.commit();c.close()
    (tmp_path/'.codex-global-state.json').write_text(json.dumps({'local-projects':{'legacy':{'name':'P','rootPaths':[str(tmp_path/'files')]}},'thread-project-assignments':{'t':{'projectId':'legacy','projectKind':'local'}},'app-server-project-id-by-legacy-project-id-by-host':{'local:C:\\old-home':{'legacy':'canonical'}}}))
    s=read_catalog(tmp_path)
    assert len(s.projects)==1 and s.conversations[0].project_id=='canonical'


def test_catalog_marks_internal_records_and_uses_recorded_parent(tmp_path):
    c=sqlite3.connect(tmp_path/'state_5.sqlite')
    c.executescript('CREATE TABLE threads(id,title,cwd,project_id,rollout_path,archived,updated_at,source,thread_source);')
    for tid,source,kind in [('user','vscode','user'),('review',json.dumps({'subagent':{'other':'guardian'}}),None),('child',json.dumps({'subagent':{'thread_spawn':{'parent_thread_id':'user'}}}),'subagent')]:
        c.execute('INSERT INTO threads VALUES (?,?,?,?,?,?,?,?,?)',(tid,tid,str(tmp_path),None,'missing',0,1,source,kind))
    c.commit();c.close()
    ts={t.id:t for t in read_catalog(tmp_path,False).conversations}
    assert not ts['user'].internal
    assert ts['review'].internal and ts['child'].internal
    assert ts['child'].parent_id=='user'
