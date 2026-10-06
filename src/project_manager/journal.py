import json
import sqlite3
from pathlib import Path


class Journal:
    def __init__(self,path):
        self.path=Path(path);self.path.parent.mkdir(parents=True,exist_ok=True)
        with self.connect() as c:
            c.executescript('CREATE TABLE IF NOT EXISTS operations(id TEXT PRIMARY KEY,state TEXT,payload TEXT); CREATE TABLE IF NOT EXISTS events(operation_id TEXT,phase TEXT,payload TEXT,created_at TEXT DEFAULT CURRENT_TIMESTAMP); CREATE TABLE IF NOT EXISTS locks(resource TEXT PRIMARY KEY,operation_id TEXT);')

    def connect(self): return sqlite3.connect(self.path,timeout=15)

    def begin(self,operation_id,payload):
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
        with self.connect() as c: row=c.execute('SELECT state,payload FROM operations WHERE id=?',(operation_id,)).fetchone()
        return {'id':operation_id,'state':row[0],'payload':json.loads(row[1])} if row else None

    def events(self,operation_id):
        with self.connect() as c:
            return [{'phase':r[0],'payload':json.loads(r[1])} for r in c.execute('SELECT phase,payload FROM events WHERE operation_id=? ORDER BY rowid',(operation_id,))]
