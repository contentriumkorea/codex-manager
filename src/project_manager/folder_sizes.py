"""Streaming read-only size scan: no file content, inventories or linked targets."""
import os,stat
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class FolderSize:
    total:int=0
    files:int=0
    skipped:int=0
    warnings:tuple=()


def measure_folders(roots,cancelled=lambda:False):
    total=files=skipped=0;warnings=[];safe=[]
    def warning(text):
        nonlocal skipped
        skipped+=1
        if len(warnings)<6:warnings.append(str(text))
    def is_link(info):return stat.S_ISLNK(info.st_mode) or bool(getattr(info,'st_file_attributes',0)&0x400)
    def check():
        if cancelled():raise InterruptedError('용량 계산을 중단했습니다.')
    for value in roots:
        check();root=Path(os.path.abspath(value))
        try:
            if root==Path(root.anchor):raise ValueError('드라이브 전체는 자동 계산하지 않습니다.')
            for ancestor in (root,*root.parents):
                if is_link(ancestor.lstat()):raise ValueError('연결 폴더 제외: '+str(root))
            if not root.is_dir():raise FileNotFoundError('폴더 없음: '+str(root))
            if any(root==r or r in root.parents for r in safe):continue
            safe=[r for r in safe if root not in r.parents];safe.append(root)
        except (OSError,ValueError) as exc:warning(exc)
    stack=list(safe)
    while stack:
        check();folder=stack.pop()
        try:
            # Recheck immediately before descending in case a folder changed to a link.
            if is_link(folder.lstat()):warning('연결 폴더 제외: '+str(folder));continue
            with os.scandir(folder) as entries:
                for entry in entries:
                    check()
                    try:
                        info=entry.stat(follow_symlinks=False)
                        if is_link(info):warning('연결 항목 제외: '+entry.path)
                        elif stat.S_ISDIR(info.st_mode):stack.append(Path(entry.path))
                        elif stat.S_ISREG(info.st_mode):total+=info.st_size;files+=1
                    except OSError as exc:warning(exc)
        except InterruptedError:raise
        except OSError as exc:warning(exc)
    return FolderSize(total,files,skipped,tuple(warnings))
