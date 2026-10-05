"""Read-only catalog including authoritative legacy desktop assignments."""
import hashlib
import json
import sqlite3
from pathlib import Path
from .models import Project, Conversation, Snapshot


def clean_path(value):
    s=str(value)
    if s.startswith('\\\\?\\UNC\\'): s='\\\\'+s[8:]
    elif s.startswith('\\\\?\\'): s=s[4:]
    return Path(s)


def load_state(home):
    path=home/'.codex-global-state.json'
    return json.loads(path.read_text(encoding='utf-8')) if path.exists() else {}


def read_catalog(home: Path) -> Snapshot:
    state=load_state(home)
    mappings=state.get('app-server-project-id-by-legacy-project-id-by-host',{}).get('local:'+str(home),{})
    projects={};conversations=[]
    databases=sorted(home.glob('state_*.sqlite'), key=lambda p:int(p.stem.split('_')[-1]), reverse=True)
    db=None
    if databases:
        source=sqlite3.connect(databases[0].as_uri()+'?mode=ro',uri=True)
        db=sqlite3.connect(':memory:');source.backup(db);source.close();db.row_factory=sqlite3.Row
        tables={r[0] for r in db.execute("select name from sqlite_master where type='table'")}
        if 'projects' in tables:
            for row in db.execute('select * from projects'):
                roots=tuple(clean_path(r[0]) for r in db.execute('select path from project_roots where project_id=? order by position',(row['id'],)))
                aliases=tuple(k for k,v in mappings.items() if v==row['id'])
                projects[row['id']]=Project(row['id'],row['name'],roots,aliases)
    for pid,p in state.get('local-projects',{}).items():
        canonical=mappings.get(pid,pid)
        if canonical not in projects:
            projects[canonical]=Project(canonical,p.get('name',pid),tuple(clean_path(x) for x in p.get('rootPaths',[])),(pid,))
    parents={}
    if db:
        if 'thread_spawn_edges' in tables:
            parents={r[1]:r[0] for r in db.execute('select parent_thread_id,child_thread_id from thread_spawn_edges')}
        for row in db.execute('select * from threads order by updated_at desc'):
            r=dict(row)
            if not r.get('has_user_event',1) and not r.get('title'): continue
            assignment=state.get('thread-project-assignments',{}).get(r['id'],{})
            pid=r.get('project_id')
            if pid is None and assignment.get('projectKind')=='local':
                pid=mappings.get(assignment.get('projectId'),assignment.get('projectId'))
            path=clean_path(r['rollout_path']) if r.get('rollout_path') else None
            runtime=()
            if path and path.exists():
                try:
                    with path.open(encoding='utf-8') as f:
                        meta=json.loads(f.readline()).get('payload',{})
                    runtime=tuple(clean_path(x) for x in meta.get('runtime_workspace_roots',[]))
                except (ValueError,OSError): pass
            revision=f"{r.get('updated_at_ms',r.get('updated_at',0))}:{path.stat().st_size if path and path.exists() else 0}"
            conversations.append(Conversation(r['id'],pid,clean_path(r['cwd']),runtime,bool(r['archived']),parents.get(r['id']),revision,False,r.get('name') or r.get('title') or r['id'],path,r.get('updated_at',0)))
        db.close()
    serialized=json.dumps({'p':[(p.id,p.name,[str(x) for x in p.roots]) for p in projects.values()],
                           't':[(t.id,t.project_id,t.revision,str(t.cwd)) for t in conversations]},sort_keys=True)
    return Snapshot(tuple(projects.values()),tuple(conversations),hashlib.sha256(serialized.encode()).hexdigest())
