import json
import os
import subprocess
import shutil
import uuid
from pathlib import Path
from ..catalog import read_catalog, load_state, clean_path
from ..models import Capabilities, Project
from ..bundles import write_json
from .rpc import CodexClient, find_cli, RpcError
from .portability import prepare_rollout


def version_tuple(value):
    import re
    match=re.search(r'(\d+)\.(\d+)\.(\d+)',value)
    return tuple(int(x) for x in match.groups()) if match else (0,0,0)


def desktop_running():
    if os.name!='nt': return False
    script="@(Get-CimInstance Win32_Process -Filter \"Name='codex.exe'\" | Where-Object { $_.CommandLine -match 'app-server' -and $_.CommandLine -notmatch '--listen' }).Count"
    try:
        r=subprocess.run(['powershell','-NoProfile','-Command',script],capture_output=True,text=True,
                         timeout=15,creationflags=subprocess.CREATE_NO_WINDOW)
        return r.returncode!=0 or int(r.stdout.strip())>0
    except (OSError,ValueError,subprocess.TimeoutExpired): return True


class CodexAdapter:
    def __init__(self, home: Path, isolated=False):
        self.home=home.resolve();self.isolated=isolated
        self._server_version=None
        if isolated and self.home==Path(os.environ.get('CODEX_HOME',str(Path.home()/'.codex'))).resolve():
            raise ValueError('격리 테스트 경로가 실제 Codex 저장소와 같습니다.')

    def snapshot(self, include_runtime=True): return read_catalog(self.home,include_runtime)

    def ensure_write_allowed(self):
        if not self.isolated and desktop_running():
            raise RuntimeError('프로젝트 연결 변경은 Codex 앱을 완전히 종료한 뒤 실행하세요. 조회와 백업은 지금 사용할 수 있습니다.')
        if version_tuple(self.server_version())<(0,160,0):
            raise RuntimeError('Codex 0.160.0 이상의 앱 실행 파일이 필요합니다. 현재 버전에서는 조회·백업만 사용할 수 있습니다.')

    def server_version(self):
        if self._server_version is None:
            self._server_version=subprocess.run([find_cli(),'--version'],capture_output=True,text=True,
                            creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0),timeout=15).stdout.strip()
        return self._server_version

    def call(self, method, params):
        return self.calls([(method,params)])[0]

    def calls(self, requests):
        self.ensure_write_allowed()
        self.home.mkdir(parents=True,exist_ok=True)
        for name in ('sessions','archived_sessions'): (self.home/name).mkdir(exist_ok=True)
        env=os.environ.copy();env['CODEX_HOME']=str(self.home)
        # Respect the real configured SQLite location. Isolated fixtures always stay local.
        if self.isolated: env['CODEX_SQLITE_HOME']=str(self.home)
        else: env.pop('CODEX_SQLITE_HOME',None)
        env['RUST_LOG']='error'
        command=[find_cli()]
        if self.isolated: command+=['-c','sqlite_home='+json.dumps(str(self.home))]
        with CodexClient(command+['app-server','--listen','stdio://'],env) as client:
            return [client.call(method,params) for method,params in requests]

    def capabilities(self):
        version=self.server_version()
        return Capabilities(version,frozenset({'project/list','project/update','thread/delete','thread/metadata/update','thread/resume'}),
            False,('복원한 대화의 이어쓰기 확인은 각 백업에 기록됩니다. 모바일 확인은 별도입니다.',))

    def create_project(self, name, roots, key):
        return self.call('project/create',{'idempotencyKey':key,'name':name,'roots':[{'path':str(p.resolve())} for p in roots]})['project']['id']

    def read_thread(self, tid, turns=False):
        return self.call('thread/read',{'threadId':tid,'includeTurns':turns})['thread']

    def assign_conversation(self, thread_id, project_id):
        self.call('thread/metadata/update',{'threadId':thread_id,'projectId':project_id or ''})
        if self.read_thread(thread_id).get('projectId')!=project_id: raise RuntimeError('대화 소속 재조회가 일치하지 않습니다.')

    def import_conversation(self, raw, tid, cwd, runtime_roots, project_id):
        self.ensure_write_allowed()
        imported=prepare_rollout(raw,self.home,cwd,runtime_roots)
        result=self.calls([
            ('thread/resume',{'threadId':tid,'path':str(imported),'cwd':str(cwd.resolve()),'runtimeWorkspaceRoots':[str(p.resolve()) for p in runtime_roots]}),
            ('thread/metadata/update',{'threadId':tid,'projectId':project_id or ''})])[0]['thread']
        if result['id']!=tid: raise RuntimeError('등록된 대화 ID가 일치하지 않습니다.')
        self.assign_conversation(tid,project_id)
        return imported

    def relocate_conversation(self, thread_id, cwd, runtime_roots):
        # App-bundled 0.160 persists world state; legacy 0.149 does not.
        self.ensure_write_allowed()
        old=self.read_thread(thread_id)
        membership=next(t.project_id for t in self.snapshot().conversations if t.id==thread_id)
        if old.get('status',{}).get('type')=='active': raise RuntimeError('실행 중인 대화는 옮길 수 없습니다.')
        self.calls([
            ('thread/resume',{'threadId':thread_id,'cwd':str(cwd.resolve()),'runtimeWorkspaceRoots':[str(p.resolve()) for p in runtime_roots]}),
            ('thread/settings/update',{'threadId':thread_id,'cwd':str(cwd.resolve())}),
            ('thread/metadata/update',{'threadId':thread_id,'projectId':membership or ''})])
        current=self.read_thread(thread_id)
        if clean_path(current['cwd']).resolve()!=cwd.resolve(): raise RuntimeError('작업 경로 변경이 저장되지 않았습니다.')
        persisted=next(t for t in self.snapshot().conversations if t.id==thread_id)
        if tuple(p.resolve() for p in persisted.runtime_roots)!=tuple(p.resolve() for p in runtime_roots):
            raise RuntimeError('작업 폴더 목록 변경이 저장되지 않았습니다.')
        self.assign_conversation(thread_id,membership)

    def update_project(self, project):
        self.call('project/update',{'projectId':project.id,'name':project.name,'roots':[{'path':str(p.resolve())} for p in project.roots]})
        updated=self.call('project/read',{'projectId':project.id})['project']
        if [clean_path(p['path']).resolve() for p in updated['roots']]!=[p.resolve() for p in project.roots]:
            raise RuntimeError('프로젝트 폴더 재조회가 일치하지 않습니다.')

    def delete_project(self, pid): self.call('project/delete',{'projectId':pid})

    def delete_thread(self, tid): self.call('thread/delete',{'threadId':tid})

    def sync_desktop_state(self, projects, assignments, removed=(),deleted=()):
        """Update legacy UI records only while the app is closed; keep all unrelated data."""
        self.ensure_write_allowed()
        if self.isolated: return
        path=self.home/'.codex-global-state.json'
        if not path.exists(): return
        before=path.read_bytes();state=json.loads(before)
        host='local:'+str(self.home)
        mapping=state.setdefault('app-server-project-id-by-legacy-project-id-by-host',{}).setdefault(host,{})
        legacy=state.setdefault('local-projects',{})
        removed_aliases={k for k,v in mapping.items() if v in removed}|set(removed)
        for pid in removed_aliases: legacy.pop(pid,None);mapping.pop(pid,None)
        project_ui={}
        for p in projects:
            aliases=[k for k,v in mapping.items() if v==p.id]
            alias=aliases[0] if aliases else p.id;project_ui[p.id]=alias;mapping[alias]=p.id
            old=legacy.get(alias,{})
            import time
            now=int(time.time()*1000)
            legacy[alias]={**old,'id':alias,'name':p.name,'rootPaths':[str(r.resolve()) for r in p.roots],
                           'createdAt':old.get('createdAt',now),'updatedAt':now}
        for tid,pid in assignments.items():
            state.setdefault('thread-project-assignments',{})[tid]={'projectKind':'local','projectId':project_ui.get(pid,pid)} if pid else None
            if not pid: state['thread-project-assignments'].pop(tid,None)
            state.get('thread-workspace-root-hints',{}).pop(tid,None)
            if tid in deleted:
                for key in ('thread-project-membership-host-ids','thread-projectless-output-directories','queued-follow-ups'):
                    if isinstance(state.get(key),dict): state[key].pop(tid,None)
                if isinstance(state.get('projectless-thread-ids'),list):
                    state['projectless-thread-ids']=[x for x in state['projectless-thread-ids'] if x!=tid]
                unread=state.get('electron-thread-read-state-v1',{}).get('unreadByIdentity',{})
                for identity in list(unread):
                    if tid in identity: unread.pop(identity,None)
        for ids in state.get('sidebar-project-thread-orders',{}).values():
            if isinstance(ids,list): ids[:]=[x for x in ids if x not in assignments]
        for key in ('project-order','pinned-project-ids'):
            if isinstance(state.get(key),list): state[key]=[p for p in state[key] if p not in removed_aliases]
        for key in ('sidebar-project-thread-orders','project-appearances'):
            if isinstance(state.get(key),dict):
                for pid in removed_aliases: state[key].pop(pid,None)
        selected=state.get('selected-project',{})
        if isinstance(selected,dict) and selected.get('projectId') in removed_aliases: state.pop('selected-project',None)
        if path.read_bytes()!=before: raise RuntimeError('앱 설정이 다른 프로세스에서 변경됐습니다. 다시 확인하세요.')
        backup=self.home/'.project-manager-state-before.json'
        backup.write_bytes(before)
        write_json(path,state)
