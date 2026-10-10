import json
import sqlite3
import uuid
from pathlib import Path


class Journal:
    def __init__(self,path):
        self.path=Path(path);self.path.parent.mkdir(parents=True,exist_ok=True)
        self.parent_report=None
        with self.connect() as c:
            c.executescript('CREATE TABLE IF NOT EXISTS operations(id TEXT PRIMARY KEY,state TEXT,payload TEXT); CREATE TABLE IF NOT EXISTS events(operation_id TEXT,phase TEXT,payload TEXT,created_at TEXT DEFAULT CURRENT_TIMESTAMP); CREATE TABLE IF NOT EXISTS locks(resource TEXT PRIMARY KEY,operation_id TEXT);')

    def connect(self): return sqlite3.connect(self.path,timeout=15)

    def begin(self,operation_id,payload):
        if self.parent_report:
            payload=dict(payload,parent_report=self.parent_report[0],parent_index=self.parent_report[1])
        with self.connect() as c:
            try:
                c.execute('INSERT INTO operations VALUES (?,?,?)',(operation_id,'planned',json.dumps(payload,default=str)))
                for resource in payload.get('resources',[]): c.execute('INSERT INTO locks VALUES (?,?)',(resource,operation_id))
            except sqlite3.IntegrityError: raise RuntimeError('같은 프로젝트의 진행·복구 작업이 있습니다.') from None

    def record(self,operation_id,phase,payload):
        with self.connect() as c:
            c.execute('INSERT INTO events(operation_id,phase,payload) VALUES (?,?,?)',(operation_id,phase,json.dumps(payload,default=str)))
            c.execute('UPDATE operations SET state=? WHERE id=?',(phase,operation_id))
            if phase in ('completed','cancelled'): c.execute('DELETE FROM locks WHERE operation_id=?',(operation_id,))

    def pending(self):
        with self.connect() as c:
            return [{'id':r[0],'state':r[1],'payload':json.loads(r[2])} for r in c.execute("SELECT id,state,payload FROM operations WHERE state NOT IN ('completed','cancelled')")]

    def operation(self,operation_id):
        with self.connect() as c:
            row=c.execute('SELECT state,payload FROM operations WHERE id=?',(operation_id,)).fetchone()
            return self.resolve_report({'id':operation_id,'state':row[0],'payload':json.loads(row[1])},c) if row else None

    def restorable(self):
        with self.connect() as c:
            records=[{'id':r[0],'state':r[1],'payload':json.loads(r[2])} for r in c.execute("SELECT id,state,payload FROM operations WHERE state='completed' ORDER BY rowid DESC")]
        return [r for r in records if (r['payload'].get('kind') in ('management-delete','rename-thread','rename-threads','file-edit') or r['payload'].get('kind')=='connections' and 'expected_project' in r['payload']) and not any(e['payload'].get('recovered') for e in self.events(r['id']))]

    def events(self,operation_id):
        with self.connect() as c:
            return [{'phase':r[0],'payload':json.loads(r[1])} for r in c.execute('SELECT phase,payload FROM events WHERE operation_id=? ORDER BY rowid',(operation_id,))]

    def history(self,home,limit=200):
        """Older journals need no migration; first event supplies the timestamp."""
        result=[]
        with self.connect() as c:
            rows=c.execute('SELECT o.id,o.state,o.payload,(SELECT MIN(created_at) FROM events e WHERE e.operation_id=o.id) FROM operations o ORDER BY o.rowid DESC')
            for key,state,payload,created_at in rows:
                data=json.loads(payload)
                if Path(data.get('home',''))!=Path(home):continue
                result.append(self.resolve_report({'id':key,'state':state,'payload':data,'created_at':created_at},c))
                if limit is not None and len(result)>=limit:break
        return result

    def report(self,payload):
        # Reports describe child operations, never hold locks or need recovery themselves.
        key=uuid.uuid4().hex
        with self.connect() as c:
            retry_of=payload.get('retry_of')
            if retry_of:
                old=c.execute('SELECT payload FROM operations WHERE id=?',(retry_of,)).fetchone()
                if not old or json.loads(old[0]).get('home')!=payload['home']:raise ValueError('재시도할 작업의 저장소를 확인하세요.')
                if c.execute("SELECT 1 FROM events WHERE operation_id=? AND json_extract(payload,'$.retry_report') IS NOT NULL",(retry_of,)).fetchone():raise ValueError('이미 재시도한 작업입니다. 최신 작업 내역을 확인하세요.')
                c.execute('INSERT INTO events(operation_id,phase,payload) VALUES (?,?,?)',(retry_of,'completed',json.dumps({'retry_report':key})))
            c.execute('INSERT INTO operations VALUES (?,?,?)',(key,'completed',json.dumps(dict(payload,kind='batch-summary'),default=str)))
            c.execute('INSERT INTO events(operation_id,phase,payload) VALUES (?,?,?)',(key,'completed','{}'))
        return key

    def update_report(self,key,payload):
        with self.connect() as c:
            c.execute('UPDATE operations SET payload=? WHERE id=? AND json_extract(payload,\'$.kind\')=\'batch-summary\'',(json.dumps(dict(payload,kind='batch-summary'),default=str),key))

    def for_item(self,report_id,index):
        child=Journal(self.path);child.parent_report=(report_id,index);return child

    def resolve_report(self,operation,c):
        """Reconcile the narrow crash gap after a child commits but before its report saves."""
        p=operation['payload']
        if p.get('kind')!='batch-summary':return operation
        for index,item in enumerate(p.get('items',[])):
            if item['status']!='in_progress':continue
            if p.get('action')=='backup':
                saved=c.execute("SELECT 1 FROM events WHERE operation_id=? AND phase='backup-verified' AND json_extract(payload,'$.index')=? LIMIT 1",(operation['id'],index)).fetchone()
                if saved or self.backup_committed(item):
                    if not saved:
                        # Save only this observation, never overwrite concurrent batch progress.
                        c.execute('INSERT INTO events(operation_id,phase,payload) VALUES (?,?,?)',(operation['id'],'backup-verified',json.dumps({'index':index})))
                    item['status']='completed';continue
            children=[(state,json.loads(payload),key) for key,state,payload in c.execute("SELECT id,state,payload FROM operations WHERE json_extract(payload,'$.parent_report')=? AND json_extract(payload,'$.parent_index')=? ORDER BY rowid",(operation['id'],index))]
            transfers=[(state,payload,key) for state,payload,key in children if payload.get('kind') in ('move','merge')]
            if transfers and transfers[-1][0]=='completed':
                recovered=c.execute("SELECT 1 FROM events WHERE operation_id=? AND json_extract(payload,'$.recovered')=1 LIMIT 1",(transfers[-1][2],)).fetchone()
                item['status']='failed' if recovered else 'completed'
            elif any(state not in ('completed','cancelled') for state,_,_ in children):item['status']='needs_recovery'
            else:item['status']='failed'
            if item['status']!='completed':item['errors']=['작업 도중 종료됐습니다. 현재 파일과 연결을 확인한 뒤 다시 시도하세요.']
        if p.get('result_state')=='running':
            p['result_state']='completed' if all(i['status']=='completed' for i in p['items']) else 'interrupted'
        return operation

    @staticmethod
    def backup_committed(item):
        # export_project publishes this proof only after full hash verification.
        # History, like other completed rows, records that check; restore re-verifies bytes.
        try:
            root=Path(item['destinations'][0]);proof=json.loads((root/'verification.json').read_text(encoding='utf-8'))
            manifest=json.loads((root/'manifest.json').read_text(encoding='utf-8'))
            return (proof.get('integrity') is True and manifest.get('state')=='complete'
                    and manifest['project']['id']==item['project_id']
                    and {Path(r['original_path']) for r in manifest['roots']}=={Path(r) for r in item['sources']})
        except (OSError,ValueError,KeyError,TypeError,IndexError):return False

    def verified_backups(self,home):
        return [Path(item['destinations'][0]) for op in self.history(home,limit=None) if op['payload'].get('action')=='backup'
                for item in op['payload'].get('items',[]) if item['status']=='completed']
