"""Public GitHub releases, verified staging, and an external Windows installer."""
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import uuid
import zipfile
import time
from http.client import IncompleteRead
from dataclasses import dataclass
from pathlib import Path
from urllib.error import HTTPError,URLError
from urllib.request import Request,urlopen
from .version import VERSION,REPOSITORY,REPOSITORY_URL,EXECUTABLE,ASSET_NAME
from .files import digest,safe_child,linked
from .bundles import write_json


@dataclass(frozen=True)
class Release:
    version:str
    available:bool
    archive_url:str
    manifest_url:str
    notes:str
    url:str


def version_number(value):
    if not re.fullmatch(r'v?\d+\.\d+\.\d+',value): raise ValueError('올바른 버전 형식이 아닙니다.')
    return tuple(map(int,value.lstrip('v').split('.')))


def parse_release(data,current=VERSION):
    if data.get('draft') or data.get('prerelease'): raise ValueError('정식 업데이트 채널의 배포가 아닙니다.')
    tag=data['tag_name'];version_number(tag)
    prefix=REPOSITORY_URL+'/releases/download/'+tag+'/'
    assets={a['name']:a['browser_download_url'] for a in data['assets']}
    urls=[assets.get(name) for name in (ASSET_NAME,'update.json')]
    if any(url!=prefix+name for url,name in zip(urls,(ASSET_NAME,'update.json'))):
        raise ValueError('공식 저장소의 업데이트 파일을 확인할 수 없습니다.')
    return Release(tag.lstrip('v'),version_number(tag)>version_number(current),*urls,data.get('body') or '',REPOSITORY_URL+'/releases/tag/'+tag)


def open_url(url):
    return urlopen(Request(url,headers={'User-Agent':'Codex-Manager/'+VERSION,'Accept':'application/vnd.github+json'}),timeout=8)


def read_json_url(url):
    with open_url(url) as response:
        data=response.read(512*1024+1)
    if len(data)>512*1024: raise ValueError('업데이트 안내 파일이 너무 큽니다.')
    return json.loads(data)


def check_release(current=VERSION):
    try: return parse_release(read_json_url('https://api.github.com/repos/'+REPOSITORY+'/releases/latest'),current)
    except HTTPError as exc:
        if exc.code==404: return None
        if exc.code not in (403,429,500,502,503,504): raise
        # Public release assets do not share the API's unauthenticated rate limit.
        metadata=read_json_url(REPOSITORY_URL+'/releases/latest/download/update.json')
        version=metadata['version'];version_number(version)
        if metadata.get('asset')!=ASSET_NAME: raise ValueError('업데이트 파일 이름이 다릅니다.')
        prefix=REPOSITORY_URL+'/releases/download/v'+version+'/'
        return Release(version,version_number(version)>version_number(current),prefix+ASSET_NAME,prefix+'update.json',
                       '최신 정식 배포입니다. 자세한 변경 내용은 GitHub 배포 페이지에서 볼 수 있습니다.',REPOSITORY_URL+'/releases/tag/v'+version)


def checked_relative(name):
    if not name or '\\' in name or ':' in name or name.startswith('/') or '..' in name.split('/'):
        raise ValueError('업데이트 파일 경로가 안전하지 않습니다.')
    for part in name.split('/'):
        if not part or part.endswith(('.', ' ')) or re.fullmatch(r'(?i)(con|prn|aux|nul|com[1-9]|lpt[1-9])(\..*)?',part):
            raise ValueError('Windows에서 사용할 수 없는 파일 경로입니다.')
    return name


def reject_links(path):
    path=Path(os.path.abspath(path))
    for ancestor in (path,*path.parents):
        if ancestor.exists() and linked(ancestor): raise ValueError('연결 폴더에서는 업데이트할 수 없습니다.')


def remove_staging(staging,parent):
    if staging.resolve().parent!=parent or not staging.name.startswith('.codex-manager-update-'):
        raise RuntimeError('임시 경로가 바뀌어 자동 정리를 중단했습니다.')
    reject_links(staging);shutil.rmtree(staging)


def validate_install_plan(plan):
    for name in ('install_dir','staged','backup','state_dir'): reject_links(plan[name])
    install=Path(plan['install_dir']).resolve();staged=Path(plan['staged']).resolve();backup=Path(plan['backup']).resolve()
    if install==Path(install.anchor) or install==staged or backup.parent!=install.parent or not backup.name.startswith('.codex-manager-previous-'):
        raise ValueError('프로그램 교체 경로가 안전하지 않습니다.')
    if staged.parent.parent!=install.parent or not staged.parent.name.startswith('.codex-manager-update-') or staged.name!='CodexManager':
        raise ValueError('준비된 프로그램 경로가 안전하지 않습니다.')
    for path in (install,staged,backup):
        for ancestor in (path,*path.parents):
            if ancestor.exists() and linked(ancestor): raise ValueError('연결 폴더에서는 프로그램을 교체할 수 없습니다.')
    if not (install/EXECUTABLE).is_file() or not (staged/EXECUTABLE).is_file(): raise ValueError('프로그램 실행 파일이 없습니다.')
    if Path(plan['state_dir']).resolve().is_relative_to(install): raise ValueError('설정 폴더를 프로그램 바깥에 두세요.')
    state=Path(plan['state_dir'])/'updates'
    for ancestor in (state,*state.parents):
        if ancestor.exists() and linked(ancestor): raise ValueError('연결된 설정 폴더에서는 업데이트할 수 없습니다.')


def stage_update(archive,metadata,release,install_dir,state_dir,parent_pid=None,cancelled=lambda:False,progress=lambda *_:None):
    reject_links(install_dir);reject_links(state_dir)
    archive=Path(archive);install_dir=Path(install_dir).resolve();state_dir=Path(state_dir).resolve()
    if install_dir==Path(install_dir.anchor) or not (install_dir/EXECUTABLE).is_file() or state_dir.is_relative_to(install_dir):
        raise ValueError('프로그램 폴더와 설정 위치를 확인하세요.')
    for ancestor in (install_dir,*install_dir.parents):
        if ancestor.exists() and linked(ancestor): raise ValueError('연결 폴더에서는 업데이트할 수 없습니다.')
    if (metadata.get('version')!=release.version or metadata.get('asset')!=ASSET_NAME or
        metadata.get('size')!=archive.stat().st_size or digest(archive)!=metadata.get('sha256')):
        raise ValueError('업데이트 다운로드 검증에 실패했습니다. 현재 프로그램을 유지합니다.')
    with zipfile.ZipFile(archive) as z:
        files={};seen=set()
        for entry in z.infolist():
            name=entry.filename.rstrip('/')
            if name=='CodexManager' and entry.is_dir(): continue
            if not name.startswith('CodexManager/'): raise ValueError('프로그램 바깥의 파일이 있습니다.')
            relative=checked_relative(name[len('CodexManager/'):])
            if relative.casefold() in seen or stat.S_ISLNK(entry.external_attr>>16): raise ValueError('중복 경로나 링크가 있습니다.')
            seen.add(relative.casefold())
            if not entry.is_dir(): files[relative]=entry
        if 'install-manifest.json' not in files: raise ValueError('프로그램 파일 목록이 없습니다.')
        if files['install-manifest.json'].file_size>512*1024: raise ValueError('파일 목록이 너무 큽니다.')
        manifest=json.loads(z.read(files['install-manifest.json']))
        expected=manifest['files']
        if manifest['version']!=release.version or set(files)!=set(expected)|{'install-manifest.json'} or EXECUTABLE not in expected:
            raise ValueError('프로그램 파일 목록이 업데이트와 다릅니다.')
        for name in expected: checked_relative(name)
        expanded=sum(e.file_size for e in files.values())
        if expanded>2*1024**3 or shutil.disk_usage(install_dir.parent).free<expanded+10*1024**2:
            raise ValueError('업데이트를 설치할 디스크 공간이 부족합니다.')
        staging=Path(tempfile.mkdtemp(prefix='.codex-manager-update-',dir=install_dir.parent))
        staged=staging/'CodexManager';staged.mkdir()
        try:
            completed=0
            for name,entry in files.items():
                if cancelled(): raise ValueError('업데이트를 취소했습니다.')
                target=safe_child(staged,name);target.parent.mkdir(parents=True,exist_ok=True)
                hasher=hashlib.sha256()
                with z.open(entry) as src,target.open('xb') as dst:
                    while chunk:=src.read(1024*1024):
                        if cancelled(): raise ValueError('업데이트를 취소했습니다.')
                        dst.write(chunk);hasher.update(chunk)
                if name in expected and hasher.hexdigest()!=expected[name]: raise ValueError('프로그램 파일 검증에 실패했습니다.')
                completed+=entry.file_size;progress(completed,expanded)
        except Exception:
            # This directory was freshly created here, under the verified parent.
            remove_staging(staging,install_dir.parent);raise
    try:
        identifier=uuid.uuid4().hex;updates=state_dir/'updates';reject_links(updates);updates.mkdir(parents=True,exist_ok=True)
        plan={'id':identifier,'version':release.version,'install_dir':str(install_dir),'staged':str(staged),
              'backup':str(install_dir.parent/('.codex-manager-previous-'+identifier)), 'state_dir':str(state_dir),
              'parent_pid':parent_pid or os.getpid(),'health':str(updates/(identifier+'.health.json')),
              'result':str(updates/'last-result.json'),'manifest_sha256':digest(staged/'install-manifest.json')}
        validate_install_plan(plan)
        return plan
    except Exception:
        remove_staging(staging,install_dir.parent);raise


def download_and_stage(release,install_dir,state_dir,progress=lambda *_:None,cancelled=lambda:False,phase=lambda *_:None):
    install_dir=Path(install_dir);reject_links(install_dir);reject_links(state_dir)
    if cancelled(): raise ValueError('업데이트를 취소했습니다.')
    metadata=read_json_url(release.manifest_url)
    if metadata.get('version')!=release.version or metadata.get('asset')!=ASSET_NAME or not isinstance(metadata.get('size'),int) or not 0<metadata['size']<2*1024**3:
        raise ValueError('올바른 업데이트 안내 파일이 아닙니다.')
    if shutil.disk_usage(install_dir.parent).free<metadata['size']+10*1024**2: raise ValueError('다운로드 공간이 부족합니다.')
    fd,name=tempfile.mkstemp(prefix='.codex-manager-download-',suffix='.zip',dir=install_dir.parent);os.close(fd);archive=Path(name)
    try:
        for attempt in range(2):
            if cancelled(): raise ValueError('업데이트를 취소했습니다.')
            phase('다운로드 중' if attempt==0 else '연결이 끊겨 다운로드를 다시 시도합니다.')
            try:
                with open_url(release.archive_url) as response,archive.open('wb') as dst:
                    done=0;progress(0,metadata['size'])
                    while chunk:=response.read(1024*1024):
                        if cancelled(): raise ValueError('업데이트를 취소했습니다.')
                        done+=len(chunk)
                        if done>metadata['size']: raise ValueError('업데이트 파일 크기가 안내와 다릅니다.')
                        dst.write(chunk);progress(done,metadata['size'])
                    if done<metadata['size']: raise IncompleteRead(b'',metadata['size']-done)
                    dst.flush();os.fsync(dst.fileno())
                break
            except (URLError,TimeoutError,ConnectionError,IncompleteRead) as exc:
                if attempt or isinstance(exc,HTTPError) and exc.code not in (429,500,502,503,504): raise
        phase('파일 검증·압축 해제 중')
        return stage_update(archive,metadata,release,install_dir,state_dir,cancelled=cancelled,progress=progress)
    finally: archive.unlink(missing_ok=True)


def launch_installer(plan):
    if not getattr(sys,'frozen',False): raise RuntimeError('실행 파일 배포에서 업데이트를 설치하세요.')
    validate_install_plan(plan)
    updates=Path(plan['state_dir'])/'updates'
    plan['id']=uuid.uuid4().hex
    plan['backup']=str(Path(plan['install_dir']).parent/('.codex-manager-previous-'+plan['id']))
    plan['health']=str(updates/(plan['id']+'.health.json'))
    plan['ready']=str(updates/(plan['id']+'.ready.json'))
    path=updates/(plan['id']+'.plan.json');write_json(path,plan)
    script=updates/(plan['id']+'.ps1');shutil.copyfile(Path(__file__).with_name('install_update.ps1'),script)
    # Stable recovery entry remains available even if power fails between renames.
    (updates/('recover-'+plan['id']+'.cmd')).write_text('@echo off\r\npowershell.exe -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File "%~dp0'+script.name+'" -Plan "%~dp0'+path.name+'"\r\n',encoding='ascii')
    process=start_helper(script,path)
    deadline=time.monotonic()+20
    while time.monotonic()<deadline:
        ready=Path(plan['ready'])
        if ready.exists():
            try:
                record=json.loads(ready.read_text(encoding='utf-8'))
                if record.get('id')==plan['id'] and record.get('version')==plan['version'] and record.get('pid')==process.pid: return
            except (ValueError,OSError): pass
        if process.poll() is not None:
            result=read_result(Path(plan['result']))
            raise RuntimeError(result.get('message','설치 도우미를 시작하지 못했습니다. 현재 프로그램을 유지합니다.'))
        time.sleep(.05)
    # Only our new helper is stopped; the running app remains open.
    process.terminate();process.wait(timeout=5)
    raise TimeoutError('설치 준비 확인 시간이 초과되었습니다. 현재 프로그램을 유지합니다.')


def start_helper(script,path):
    return subprocess.Popen([str(Path(os.environ['SystemRoot'])/'System32/WindowsPowerShell/v1.0/powershell.exe'),'-NoProfile','-WindowStyle','Hidden','-ExecutionPolicy','Bypass','-File',str(script),'-Plan',str(path)],
                     cwd=path.parent,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0),close_fds=True)


def read_result(path):
    try:
        data=json.loads(path.read_text(encoding='utf-8'));return data if isinstance(data,dict) else {}
    except (ValueError,OSError): return {}


def prepared_path(install_dir,state_dir):
    key=hashlib.sha256(str(Path(install_dir).resolve()).casefold().encode()).hexdigest()[:16]
    return Path(state_dir)/'updates'/('prepared-'+key+'.json')


def store_prepared(plan):
    path=prepared_path(plan['install_dir'],plan['state_dir']);path.parent.mkdir(parents=True,exist_ok=True);write_json(path,plan)


def load_prepared(release,install_dir,state_dir,cancelled=lambda:False):
    path=prepared_path(install_dir,state_dir);plan=read_result(path)
    if plan.get('version')!=release.version or plan.get('install_dir')!=str(Path(install_dir).resolve()): return None
    try:
        validate_install_plan(plan);staged=Path(plan['staged']);manifest=staged/'install-manifest.json'
        if digest(manifest)!=plan['manifest_sha256']: return None
        for name,expected in json.loads(manifest.read_text(encoding='utf-8'))['files'].items():
            if cancelled(): raise InterruptedError('업데이트를 취소했습니다.')
            if digest(safe_child(staged,name))!=expected: return None
        plan['parent_pid']=os.getpid();return plan
    except InterruptedError: raise
    except (ValueError,OSError,KeyError,TypeError): return None


def pending_updates(state_dir):
    pending=[]
    for status in (Path(state_dir)/'updates').glob('*.status.json'):
        try:
            record=json.loads(status.read_text(encoding='utf-8'))
            plan=status.with_name(status.name.replace('.status.json','.plan.json'))
            if record['phase'] in ('replacing','starting') and plan.is_file(): pending.append(plan)
        except (OSError,ValueError,KeyError): continue
    return pending
