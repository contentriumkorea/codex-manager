"""Bounded, cancellable discovery of separate Codex homes, without reading auth."""
import os,re,sqlite3,time
from dataclasses import dataclass
from pathlib import Path
from contextlib import closing
from .catalog import sqlite_directory
from .settings import read_settings


@dataclass(frozen=True)
class Store:
    home:Path
    available:bool
    projects:int=0
    chats:int=0
    error:str=''


@dataclass(frozen=True)
class StoreScan:
    stores:tuple
    visited:int
    limited:bool


SKIP={'.git','.venv','node_modules','.test-artifacts','__pycache__','windows','program files','program files (x86)','appdata','$recycle.bin','system volume information','build','dist','releases','.project-manager-recovery'}


def store_info(home,cancelled=lambda:False):
    home=Path(home).resolve()
    try:
        if cancelled():raise InterruptedError('저장소 검색을 중단했습니다.')
        names={p.name for p in home.iterdir()}
        if not ('.codex-global-state.json' in names or any(re.fullmatch(r'state_\d+\.sqlite',n) for n in names) or
                ('sessions' in names or 'archived_sessions' in names) and ('config.toml' in names or 'auth.json' in names)):
            return Store(home,False,error='Codex 대화 저장소를 찾을 수 없습니다.')
        databases=sorted(sqlite_directory(home).glob('state_*.sqlite'),key=lambda p:int(p.stem.split('_')[-1]),reverse=True)
        projects=len(read_settings(home/'.codex-global-state.json').get('local-projects',{}));chats=0
        if databases:
            with closing(sqlite3.connect(databases[0].as_uri()+'?mode=ro',uri=True,timeout=1)) as connection:
                deadline=time.monotonic()+3
                connection.set_progress_handler(lambda:1 if cancelled() or time.monotonic()>deadline else 0,1000)
                tables={r[0] for r in connection.execute("select name from sqlite_master where type='table'")}
                if 'projects' in tables:projects=max(projects,connection.execute('select count(*) from projects').fetchone()[0])
                if 'threads' in tables:chats=connection.execute('select count(*) from threads').fetchone()[0]
        if cancelled():raise InterruptedError('저장소 검색을 중단했습니다.')
        return Store(home,True,projects,chats)
    except InterruptedError:raise
    except Exception as exc:
        if cancelled():raise InterruptedError('저장소 검색을 중단했습니다.')
        return Store(home,False,error=str(exc))


def default_search_roots(homes):
    roots=[Path.home(),*(Path(h).parent for h in homes)]
    for letter in 'CDEFG':
        drive=Path(letter+':/')
        if os.name=='nt' and drive.is_dir():roots.append(drive)
    return tuple(dict.fromkeys(roots))


def discover_stores(known,search_roots,cancelled=lambda:False,limit=12000,depth=5):
    found={};seen=set();visited=0;limited=False
    def check():
        if cancelled():raise InterruptedError('저장소 검색을 중단했습니다.')
    for path in known:
        check();info=store_info(path,cancelled);found[info.home]=info
    stack=[(Path(p),0) for p in reversed(search_roots)]
    while stack:
        check()
        if visited>=limit:limited=True;break
        path,level=stack.pop()
        try:
            info=path.lstat()
            if path.is_symlink() or getattr(info,'st_file_attributes',0)&0x400:continue
            key=os.path.normcase(os.path.abspath(path))
            if key in seen:continue
            seen.add(key);visited+=1
            with os.scandir(path) as iterator:entries=list(iterator)
            names={entry.name for entry in entries}
            candidate=('.codex-global-state.json' in names or any(re.fullmatch(r'state_\d+\.sqlite',n) for n in names) or
                       ('sessions' in names or 'archived_sessions' in names) and ('config.toml' in names or 'auth.json' in names))
            if candidate:
                info=store_info(path,cancelled)
                if info.available:found[info.home]=info
                continue
            if level<depth:
                for entry in entries:
                    check()
                    if entry.name.lower() not in SKIP and entry.is_dir(follow_symlinks=False):stack.append((Path(entry.path),level+1))
        except InterruptedError:raise
        except OSError:continue
    return StoreScan(tuple(sorted(found.values(),key=lambda s:str(s.home).casefold())),visited,limited)
