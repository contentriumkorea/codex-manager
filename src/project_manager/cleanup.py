from pathlib import Path
from .files import scan_roots, digest, safe_child, validate_paths, linked
from .models import Verification


def remove_verified_files(inventory,sources,destinations):
    errors=validate_paths(tuple(sources.values()),tuple(destinations.values()),())
    if errors: return Verification(False,errors)
    current=scan_roots(sources,True)
    def signatures(inv): return sorted((e.root_id,e.relative_path,e.size,e.sha256) for e in inv.entries)
    if current.blockers or signatures(current)!=signatures(inventory) or set(current.directories)!=set(inventory.directories):
        return Verification(False,('원본 파일이나 폴더가 변경됐습니다. 원본 정리를 중단했습니다.',))
    try:
        for e in inventory.entries:
            target=safe_child(destinations[e.root_id],e.relative_path)
            if not target.is_file() or digest(target)!=e.sha256: raise ValueError(f'대상 파일 검증 실패: {target}')
        for e in inventory.entries:
            source=safe_child(sources[e.root_id],e.relative_path)
            # Recheck immediately before unlink. Never recursively delete a computed path.
            if digest(source)!=e.sha256: raise ValueError(f'삭제 직전 원본 변경: {source}')
            source.unlink()
        for rid,relative in sorted(inventory.directories,key=lambda x:len(Path(x[1]).parts),reverse=True):
            path=safe_child(sources[rid],relative)
            if path.exists(): path.rmdir()
        for root in sources.values():
            if linked(root): raise ValueError('원본 루트가 링크로 바뀌었습니다.')
            root.rmdir()
        return Verification(True)
    except (OSError,ValueError) as exc: return Verification(False,(str(exc),))
