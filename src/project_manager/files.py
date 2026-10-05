import hashlib
import os
import shutil
import stat
import uuid
from pathlib import Path
from .models import FileEntry, Inventory, Verification


def digest(path: Path) -> str:
    h=hashlib.sha256()
    with path.open('rb') as f:
        for chunk in iter(lambda:f.read(1024*1024),b''): h.update(chunk)
    return h.hexdigest()


def linked(path: Path):
    s=path.lstat()
    return path.is_symlink() or bool(getattr(s,'st_file_attributes',0)&getattr(stat,'FILE_ATTRIBUTE_REPARSE_POINT',0x400))


def inside(child, parent):
    return child==parent or parent in child.parents


def validate_paths(sources, destinations, protected):
    errors=[]
    src=[p.resolve() for p in sources];dst=[p.resolve() for p in destinations]
    for p in (*src,*dst):
        if p==Path(p.anchor): errors.append(f'드라이브 루트는 사용할 수 없습니다: {p}')
        if any(inside(p,q.resolve()) or inside(q.resolve(),p) for q in protected): errors.append(f'보호된 저장소 경로: {p}')
        for ancestor in (p,*p.parents):
            if ancestor.exists() and linked(ancestor):
                errors.append(f'링크 경로는 자동 처리할 수 없습니다: {ancestor}');break
    for a in src:
        for b in dst:
            if inside(a,b) or inside(b,a): errors.append(f'원본과 대상이 겹칩니다: {a} / {b}')
    for group in (src,dst):
        for i,a in enumerate(group):
            for b in group[i+1:]:
                if inside(a,b) or inside(b,a): errors.append(f'루트 폴더가 겹칩니다: {a} / {b}')
    return tuple(dict.fromkeys(errors))


def safe_child(root: Path, relative: str) -> Path:
    rel=Path(relative)
    if rel.is_absolute() or rel.drive or '..' in rel.parts or ':' in relative or '\\' in relative:
        raise ValueError(f'허용되지 않는 상대 경로: {relative}')
    target=root/rel
    if not inside(target.resolve(),root.resolve()): raise ValueError('목적지 범위를 벗어나는 경로입니다.')
    for p in (target,*target.parents):
        if p.exists() and linked(p): raise ValueError(f'링크를 따라갈 수 없습니다: {p}')
        if p==root: break
    return target


def scan_roots(roots, hash_files):
    entries=[];directories=[];errors=list(validate_paths(tuple(roots.values()),(),()))
    seen=set();logical=0
    for rid,root in roots.items():
        if not root.is_dir(): errors.append(f'폴더를 찾을 수 없습니다: {root}');continue
        try:
            for folder, dirs, files in os.walk(root,followlinks=False,onerror=lambda exc:errors.append(str(exc))):
                for name in list(dirs):
                    p=Path(folder)/name
                    if linked(p): errors.append(f'링크 폴더: {p}');dirs.remove(name)
                    else: directories.append((rid,p.relative_to(root).as_posix()))
                for name in files:
                    p=Path(folder)/name
                    try:
                        if linked(p): raise ValueError(f'링크 또는 클라우드 자리표시자: {p}')
                        before=p.stat();file_id=f'{before.st_dev}:{before.st_ino}'
                        sha=digest(p) if hash_files else None
                        after=p.stat()
                        if (before.st_mtime_ns,before.st_size)!=(after.st_mtime_ns,after.st_size): raise ValueError(f'검사 중 파일 변경: {p}')
                        entries.append(FileEntry(rid,p.relative_to(root).as_posix(),before.st_size,sha,file_id,before.st_mtime_ns))
                        if file_id not in seen: logical+=before.st_size;seen.add(file_id)
                    except (OSError,ValueError) as exc: errors.append(str(exc))
        except (OSError,ValueError) as exc: errors.append(str(exc))
    return Inventory(tuple(entries),tuple(errors),logical,None,tuple(directories))


def copy_verified(inventory,sources,destinations,progress,cancelled):
    if inventory.blockers: return Verification(False,inventory.blockers)
    problems=validate_paths(tuple(sources.values()),tuple(destinations.values()),())
    if problems: return Verification(False,problems)
    total=sum(e.size for e in inventory.entries);done=0
    try:
        for rid,destination in destinations.items(): destination.mkdir(parents=True,exist_ok=True)
        for rid,relative in inventory.directories: safe_child(destinations[rid],relative).mkdir(parents=True,exist_ok=True)
        for e in inventory.entries:
            if cancelled(): raise ValueError('사용자가 작업을 취소했습니다.')
            source=safe_child(sources[e.root_id],e.relative_path)
            target=safe_child(destinations[e.root_id],e.relative_path)
            s=source.stat()
            if s.st_size!=e.size or s.st_mtime_ns!=e.mtime_ns or (e.sha256 and digest(source)!=e.sha256): raise ValueError(f'원본이 변경됐습니다: {source}')
            target.parent.mkdir(parents=True,exist_ok=True)
            if target.exists():
                if e.sha256 and digest(target)==e.sha256: done+=e.size;progress(done,total);continue
                raise FileExistsError(f'파일 충돌: {target}')
            if shutil.disk_usage(target.parent).free < e.size+1024*1024: raise OSError('대상 디스크 공간이 부족합니다.')
            temporary=target.with_name(target.name+'.'+uuid.uuid4().hex+'.partial')
            try:
                h=hashlib.sha256()
                with source.open('rb') as src,temporary.open('xb') as dst:
                    for chunk in iter(lambda:src.read(1024*1024),b''):
                        if cancelled(): raise ValueError('사용자가 작업을 취소했습니다.')
                        dst.write(chunk);h.update(chunk);done+=len(chunk);progress(done,total)
                    dst.flush();os.fsync(dst.fileno())
                if e.sha256 and h.hexdigest()!=e.sha256: raise ValueError('복사 중 원본 내용이 변경됐습니다.')
                if digest(temporary)!=h.hexdigest(): raise ValueError('복사본 검증에 실패했습니다.')
                shutil.copystat(source,temporary)
                # Windows rename refuses to overwrite. On other platforms use link+unlink.
                if os.name=='nt': temporary.rename(target)
                else: os.link(temporary,target);temporary.unlink()
            finally: temporary.unlink(missing_ok=True)
        return Verification(True)
    except (OSError,ValueError,KeyError) as exc: return Verification(False,(str(exc),))
