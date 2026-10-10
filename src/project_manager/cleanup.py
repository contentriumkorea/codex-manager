from pathlib import Path
from .files import scan_roots, digest, safe_child, validate_paths, linked
from .models import Verification


def cleanup_export(project,bundle,adapter,journal):
    import json,uuid
    from .bundles import verify_bundle,load_manifest
    from .models import OperationResult
    adapter.ensure_write_allowed()
    check=verify_bundle(bundle)
    if not check.ok: raise ValueError('\n'.join(check.errors))
    m=load_manifest(bundle)
    proof=json.loads((bundle/'verification.json').read_text(encoding='utf-8'))
    if not proof.get('restore_verified') or proof.get('bundle_id')!=m['bundle_id'] or proof.get('manifest_digest')!=digest(bundle/'manifest.json'):
        raise ValueError('복원 후 이어쓰기 검증이 기록된 백업이 필요합니다.')
    if m['project']['id']!=project.id: raise ValueError('선택한 프로젝트의 백업이 아닙니다.')
    if m['dependencies']: raise ValueError('외부 의존성이 있어 원본을 정리할 수 없습니다.')
    snapshot=adapter.snapshot()
    actual_project=next((p for p in snapshot.projects if p.id==project.id),None)
    paths=lambda roots:tuple(Path(r).resolve() for r in roots)
    if not actual_project or paths(actual_project.roots)!=paths(m['project']['roots']):
        raise ValueError('프로젝트 폴더 연결이 백업 이후 바뀌었습니다. 새 백업을 만드세요.')
    tids={t['id'] for t in m['conversations']}
    current=[t for t in snapshot.conversations if t.project_id==project.id]
    affected={t.id for t in current}
    while True:
        children={t.id for t in snapshot.conversations if t.parent_id in affected}
        if children<=affected: break
        affected.update(children)
    from .grouping import display_membership
    from .settings import read_settings
    inferred=display_membership(snapshot,read_settings(adapter.home/'.codex-global-state.json')).inferred
    if any(pid==project.id and tid not in affected for tid,pid in inferred.items()):
        raise ValueError('폴더 기준 대화의 연결이 아직 확정되지 않았습니다. 새로고침하고 연결을 확정한 뒤 새 백업을 만드세요.')
    if affected!=tids: raise ValueError('대화 목록이 백업 이후 바뀌었습니다. 새 백업을 만드세요.')
    expected={x['path']:x for x in (json.loads(line) for line in (bundle/'checksums.jsonl').read_text(encoding='utf-8').splitlines())}
    for t in snapshot.conversations:
        if t.id in tids:
            original=next(r for r in m['conversations'] if r['id']==t.id)
            if (t.project_id!=original['project_id'] or t.parent_id!=original['parent_id'] or
                t.cwd.resolve()!=Path(original['cwd']).resolve() or paths(t.runtime_roots)!=paths(original['runtime_roots']) or
                t.archived!=original['archived'] or t.attachments):
                raise ValueError('대화 소속·부모·작업 폴더가 백업 이후 바뀌었습니다. 새 백업을 만드세요.')
            key=original['rollout']
            if not t.rollout or digest(t.rollout)!=expected[key]['sha256']: raise ValueError('백업 이후 대화가 바뀌었습니다. 원본 정리를 중단했습니다.')
    sources={r['id']:Path(r['original_path']) for r in m['roots']}
    backup_roots={r['id']:bundle/r['bundle_path'] for r in m['roots']}
    inv=scan_roots(sources,True)
    signatures=lambda entries:sorted((e['root_id'],e['relative_path'],e['size'],e['sha256']) for e in entries)
    from dataclasses import asdict
    if inv.blockers or signatures([asdict(e) for e in inv.entries])!=signatures(m['inventory']) or set(inv.directories)!=set(tuple(x) for x in m['directories']):
        raise ValueError('원본 파일이 바뀌었습니다. 원본 정리를 중단했습니다.')
    if validate_paths(tuple(sources.values()),tuple(backup_roots.values()),(adapter.home,)):
        raise ValueError('원본 정리에 사용할 경로가 안전하지 않습니다.')
    for p in snapshot.projects:
        if p.id!=project.id and any(r.resolve().is_relative_to(s.resolve()) or s.resolve().is_relative_to(r.resolve()) for r in p.roots for s in sources.values()):
            raise ValueError(f'다른 프로젝트와 폴더를 공유합니다: {p.name}')
    operation_id=uuid.uuid4().hex
    journal.begin(operation_id,{'kind':'export-cleanup','home':str(adapter.home),'resources':[project.id],'bundle':str(bundle),'source':project.id,
                                'bundle_id':m['bundle_id'],'manifest_digest':digest(bundle/'manifest.json'),'project':asdict(actual_project)})
    try:
        journal.record(operation_id,'cleaning',{})
        child_ids={t.id for t in snapshot.conversations if t.id in tids and t.parent_id in tids}
        for tid in sorted(tids-child_ids):
            journal.record(operation_id,'deleting',{'thread_id':tid});adapter.delete_thread(tid)
        after=adapter.snapshot()
        if any(t.id in tids for t in after.conversations): raise ValueError('일부 대화가 아직 남아 있습니다.')
        check=remove_verified_files(inv,sources,backup_roots)
        if not check.ok: raise ValueError('\n'.join(check.errors))
        adapter.delete_project(project.id)
        adapter.sync_desktop_state([],{tid:None for tid in tids},[project.id],deleted=tids)
        journal.record(operation_id,'completed',{})
        return OperationResult('completed','백업 검증 완료','대화 삭제 완료','원본 파일 정리 완료','모바일 미확인',report_id=operation_id)
    except Exception as exc:
        journal.record(operation_id,'needs_recovery',{'error':str(exc)})
        return OperationResult('needs_recovery','백업 유지','정리 일부 완료','작업 기록 확인 필요','모바일 미확인',(str(exc),),report_id=operation_id)


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
