"""Read-only catalog including authoritative legacy desktop assignments."""
import hashlib
import json
import sqlite3
import time
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


_runtime_cache={}


def reverse_lines(stream,size):
    """Linear-time reverse reading, including multi-megabyte image records."""
    pos=size;fragments=[]
    while pos:
        count=min(pos,65536);pos-=count;stream.seek(pos)
        parts=stream.read(count).split(b'\n')
        if len(parts)==1:
            fragments.append(parts[0]);continue
        yield parts[-1]+b''.join(reversed(fragments))
        fragments=[]
        yield from reversed(parts[1:-1])
        fragments.append(parts[0])
    if fragments: yield b''.join(reversed(fragments))


def active_runtime_roots(path):
    """Latest persisted settings win; read backwards without loading large histories."""
    stat=path.stat();key=(str(path),stat.st_size,stat.st_mtime_ns)
    if key in _runtime_cache: return _runtime_cache[key]
    with path.open('rb') as f:
        result=()
        for line in reverse_lines(f,stat.st_size):
            if b'"runtime_workspace_roots"' not in line: continue
            item=json.loads(line);payload=item.get('payload',{})
            if item.get('type') in ('session_meta','turn_context'): settings=payload
            elif item.get('type')=='event_msg' and payload.get('type')=='thread_settings_applied': settings=payload.get('thread_settings',{})
            else: continue
            if 'runtime_workspace_roots' in settings:
                result=tuple(clean_path(x) for x in settings['runtime_workspace_roots'] or [])
                break
    if len(_runtime_cache)>4096: _runtime_cache.clear()
    _runtime_cache[key]=result
    return result


def read_catalog(home: Path, include_runtime=True) -> Snapshot:
    state=load_state(home)
    mappings=state.get('app-server-project-id-by-legacy-project-id-by-host',{}).get('local:'+str(home),{})
    projects={};conversations=[]
    databases=sorted(home.glob('state_*.sqlite'), key=lambda p:int(p.stem.split('_')[-1]), reverse=True)
    db=None
    if databases:
        source=sqlite3.connect(databases[0].as_uri()+'?mode=ro',uri=True)
        db=sqlite3.connect(':memory:');deadline=time.monotonic()+10
        def backup_progress(*_):
            if time.monotonic()>deadline: raise RuntimeError('Codex 데이터베이스가 사용 중입니다. 잠시 후 다시 조회하세요.')
        try: source.backup(db,pages=1024,progress=backup_progress)
        finally: source.close()
        db.row_factory=sqlite3.Row
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
            assignment=state.get('thread-project-assignments',{}).get(r['id'],{})
            pid=r.get('project_id')
            if pid is None and assignment.get('projectKind')=='local':
                pid=mappings.get(assignment.get('projectId'),assignment.get('projectId'))
            path=clean_path(r['rollout_path']) if r.get('rollout_path') else None
            runtime=()
            if include_runtime and path and path.exists():
                try:
                    runtime=active_runtime_roots(path)
                except (ValueError,OSError): pass
            revision=f"{r.get('updated_at_ms',r.get('updated_at',0))}:{path.stat().st_size if path and path.exists() else 0}"
            attachments=tuple(dict(x) for x in db.execute('select * from thread_attachments where thread_id=?',(r['id'],))) if include_runtime and 'thread_attachments' in tables else ()
            conversations.append(Conversation(r['id'],pid,clean_path(r['cwd']),runtime,bool(r['archived']),parents.get(r['id']),revision,False,r.get('name') or r.get('title') or r['id'],path,r.get('updated_at',0),attachments))
        db.close()
    serialized=json.dumps({'p':[(p.id,p.name,[str(x) for x in p.roots]) for p in projects.values()],
                           't':[(t.id,t.project_id,t.revision,str(t.cwd),[str(p) for p in t.runtime_roots],t.parent_id,t.attachments) for t in conversations]},sort_keys=True)
    return Snapshot(tuple(projects.values()),tuple(conversations),hashlib.sha256(serialized.encode()).hexdigest())
