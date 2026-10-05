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
