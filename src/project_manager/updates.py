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
from dataclasses import dataclass
from pathlib import Path
from urllib.error import HTTPError
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
    return urlopen(Request(url,headers={'User-Agent':'Codex-Manager/'+VERSION,'Accept':'application/vnd.github+json'}),timeout=15)


def read_json_url(url):
    with open_url(url) as response:
        data=response.read(512*1024+1)
    if len(data)>512*1024: raise ValueError('업데이트 안내 파일이 너무 큽니다.')
    return json.loads(data)


def check_release(current=VERSION):
    try: return parse_release(read_json_url('https://api.github.com/repos/'+REPOSITORY+'/releases/latest'),current)
    except HTTPError as exc:
        if exc.code==404: return None
        raise


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


def stage_update(archive,metadata,release,install_dir,state_dir,parent_pid=None,cancelled=lambda:False):
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
            for name,entry in files.items():
                if cancelled(): raise ValueError('업데이트를 취소했습니다.')
                target=safe_child(staged,name);target.parent.mkdir(parents=True,exist_ok=True)
                with z.open(entry) as src,target.open('xb') as dst: shutil.copyfileobj(src,dst,1024*1024)
                if name in expected and digest(target)!=expected[name]: raise ValueError('프로그램 파일 검증에 실패했습니다.')
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


def download_and_stage(release,install_dir,state_dir,progress=lambda *_:None,cancelled=lambda:False):
    metadata=read_json_url(release.manifest_url)
    if metadata.get('version')!=release.version or metadata.get('asset')!=ASSET_NAME or not isinstance(metadata.get('size'),int) or not 0<metadata['size']<2*1024**3:
        raise ValueError('올바른 업데이트 안내 파일이 아닙니다.')
    if shutil.disk_usage(install_dir.parent).free<metadata['size']+10*1024**2: raise ValueError('다운로드 공간이 부족합니다.')
    fd,name=tempfile.mkstemp(prefix='.codex-manager-download-',suffix='.zip',dir=install_dir.parent);os.close(fd);archive=Path(name)
    try:
        with open_url(release.archive_url) as response,archive.open('wb') as dst:
            done=0
            while chunk:=response.read(1024*1024):
                if cancelled(): raise ValueError('업데이트를 취소했습니다.')
                done+=len(chunk)
                if done>metadata['size']: raise ValueError('업데이트 파일 크기가 안내와 다릅니다.')
                dst.write(chunk);progress(done,metadata['size'])
            dst.flush();os.fsync(dst.fileno())
        return stage_update(archive,metadata,release,install_dir,state_dir,cancelled=cancelled)
    finally: archive.unlink(missing_ok=True)


def launch_installer(plan):
    if not getattr(sys,'frozen',False): raise RuntimeError('실행 파일 배포에서 업데이트를 설치하세요.')
    validate_install_plan(plan)
    updates=Path(plan['state_dir'])/'updates'
    path=updates/(plan['id']+'.plan.json');write_json(path,plan)
    script=updates/(plan['id']+'.ps1');shutil.copyfile(Path(__file__).with_name('install_update.ps1'),script)
    # Stable recovery entry remains available even if power fails between renames.
    (updates/('recover-'+plan['id']+'.cmd')).write_text('@echo off\r\npowershell.exe -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File "%~dp0'+script.name+'" -Plan "%~dp0'+path.name+'"\r\n',encoding='ascii')
    start_helper(script,path)


def start_helper(script,path):
    subprocess.Popen([str(Path(os.environ['SystemRoot'])/'System32/WindowsPowerShell/v1.0/powershell.exe'),'-NoProfile','-WindowStyle','Hidden','-ExecutionPolicy','Bypass','-File',str(script),'-Plan',str(path)],
                     cwd=path.parent,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0),close_fds=True)


def pending_updates(state_dir):
    pending=[]
    for status in (Path(state_dir)/'updates').glob('*.status.json'):
        try:
            record=json.loads(status.read_text(encoding='utf-8'))
            plan=status.with_name(status.name.replace('.status.json','.plan.json'))
            if record['phase'] in ('replacing','starting') and plan.is_file(): pending.append(plan)
        except (OSError,ValueError,KeyError): continue
    return pending
