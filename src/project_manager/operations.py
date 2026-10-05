import json
import shutil
import uuid
from dataclasses import asdict
from pathlib import Path
from .models import FileEntry, Inventory, OperationResult, Project
from .files import copy_verified, digest, scan_roots, safe_child, validate_paths, inside
from .bundles import load_manifest, verify_bundle, write_json
from .catalog import clean_path


def require(verification):
    if not verification.ok: raise ValueError('\n'.join(verification.errors))


def map_path(path, mapping):
    for old,new in sorted(mapping.items(),key=lambda x:len(str(x[0])),reverse=True):
        if inside(path.resolve(),old.resolve()): return new/path.resolve().relative_to(old.resolve())
    raise ValueError(f'프로젝트 폴더 밖의 작업 경로입니다: {path}')


def transfer_project(adapter, project, destinations, journal, recovery_root, target=None, clean=False, progress=lambda *_:None, cancelled=lambda:False):
    adapter.ensure_write_allowed()
    snapshot=adapter.snapshot()
    source=next(p for p in snapshot.projects if p.id==project.id)
    conversations=[t for t in snapshot.conversations if t.project_id==source.id]
    selected={t.id for t in conversations}
    children=[t for t in snapshot.conversations if t.parent_id in selected]
    if children: raise ValueError('자식 대화가 있는 프로젝트는 현재 이동 검증 대상에서 제외됩니다. 백업은 자식 대화를 함께 보존합니다.')
    if target and target.id==source.id: raise ValueError('같은 프로젝트에 합칠 수 없습니다.')
    roots={f'root-{i+1:02d}':r for i,r in enumerate(source.roots)}
    dest={rid:Path(destinations[str(root)]).resolve() for rid,root in roots.items()}
    mapping={root:dest[rid] for rid,root in roots.items()}
    errors=validate_paths(tuple(roots.values()),tuple(dest.values()),(adapter.home,))
    if errors: raise ValueError('\n'.join(errors))
    for p in snapshot.projects:
        if p.id in (source.id,target.id if target else None): continue
        if any(inside(root.resolve(),r.resolve()) or inside(r.resolve(),root.resolve()) for root in source.roots for r in p.roots):
            raise ValueError(f'다른 프로젝트와 공유하는 폴더입니다: {p.name}')
    for t in conversations:
        map_path(t.cwd,mapping)
        for r in t.runtime_roots: map_path(r,mapping)
    inventory=scan_roots(roots,True)
    if inventory.blockers: raise ValueError('\n'.join(inventory.blockers))
    if clean and not adapter.isolated: raise ValueError('실제 계정에서 복원 후 이어쓰기 검증 전에는 원본 정리를 사용할 수 없습니다.')
    operation_id=uuid.uuid4().hex
    recovery=Path(recovery_root)/operation_id;recovery.mkdir(parents=True)
    journal.begin(operation_id,{'kind':'merge' if target else 'move','resources':[source.id]+([target.id] if target else []),
                                'source':asdict(source),'destinations':destinations,'recovery':str(recovery)})
    moved=[];project_changed=False
    try:
        for t in conversations:
            if not t.rollout or not t.rollout.exists(): raise ValueError(f'대화 원본이 없습니다: {t.title}')
            saved=recovery/f'{t.id}.jsonl';shutil.copyfile(t.rollout,saved)
            if digest(saved)!=digest(t.rollout): raise ValueError('복구 사본 검증 실패')
        write_json(recovery/'metadata.json',{'project':asdict(source),'target':asdict(target) if target else None,'conversations':[asdict(t) for t in conversations],'destinations':destinations})
        journal.record(operation_id,'copying',{})
        require(copy_verified(inventory,roots,dest,progress,cancelled))
        journal.record(operation_id,'verified',{'inventory':[asdict(e) for e in inventory.entries]})
        if cancelled(): raise ValueError('작업을 취소했습니다. 원본을 유지했습니다.')
        current=adapter.snapshot()
        if current.revision!=snapshot.revision: raise ValueError('대화 또는 프로젝트가 변경됐습니다. 원본을 유지하고 다시 계획하세요.')
        new_project=Project(target.id,target.name,target.roots,target.legacy_ids) if target else Project(source.id,source.name,tuple(dest.values()),source.legacy_ids)
        if target:
            extra=[p for p in dest.values() if not any(inside(p,r.resolve()) for r in new_project.roots)]
            new_project=Project(new_project.id,new_project.name,new_project.roots+tuple(extra),new_project.legacy_ids)
        journal.record(operation_id,'relinking',{})
        adapter.update_project(new_project);project_changed=True
        for t in conversations:
            journal.record(operation_id,'relinking',{'thread_id':t.id,'old_cwd':str(t.cwd),'new_cwd':str(map_path(t.cwd,mapping))})
            adapter.relocate_conversation(t.id,map_path(t.cwd,mapping),tuple(map_path(r,mapping) for r in t.runtime_roots) or (map_path(t.cwd,mapping),))
            moved.append(t)
            adapter.assign_conversation(t.id,new_project.id)
        journal.record(operation_id,'linked',{})
        removed=[]
        if target:
            remaining=[t for t in adapter.snapshot().conversations if t.project_id==source.id]
            if remaining: raise ValueError('원본 프로젝트에 대화가 남아 있어 정리를 중단했습니다.')
            adapter.delete_project(source.id);removed=[source.id]
        adapter.sync_desktop_state([new_project],{t.id:new_project.id for t in conversations},removed)
        if clean:
            from .cleanup import remove_verified_files
            require(remove_verified_files(inventory,roots,dest))
        journal.record(operation_id,'completed',{})
        return OperationResult('completed','검증 완료','연결 변경 완료','원본 정리 완료' if clean else '원본 파일 유지','모바일 미확인')
    except Exception as exc:
        rollback_errors=[]
        for t in reversed(moved):
            try:
                adapter.relocate_conversation(t.id,t.cwd,t.runtime_roots or (t.cwd,));adapter.assign_conversation(t.id,source.id)
            except Exception as error: rollback_errors.append(str(error))
        if project_changed and not target:
            try: adapter.update_project(source)
            except Exception as error: rollback_errors.append(str(error))
        journal.record(operation_id,'needs_recovery',{'error':str(exc),'rollback_errors':rollback_errors})
        return OperationResult('needs_recovery','복사본 유지','복구 확인 필요','원본 파일 유지','모바일 미확인',(str(exc),*rollback_errors))


def import_bundle(bundle, destinations, adapter, journal, progress=lambda *_:None, cancelled=lambda:False):
    adapter.ensure_write_allowed();require(verify_bundle(bundle));m=load_manifest(bundle)
    if m['dependencies']: raise ValueError('외부 첨부·자식 대화·worktree 의존성을 해결한 뒤 가져올 수 있습니다.')
    snapshot=adapter.snapshot();existing={t.id:t for t in snapshot.conversations}
    conflicts=[t['id'] for t in m['conversations'] if t['id'] in existing]
    if conflicts: raise ValueError('대상 컴퓨터에 같은 ID의 대화가 있습니다. 기존 대화를 덮어쓰지 않습니다: '+', '.join(conflicts))
    src={r['id']:safe_child(bundle,r['bundle_path']) for r in m['roots']}
    dest={rid:Path(destinations[rid]).resolve() for rid in src}
    errors=validate_paths(tuple(src.values()),tuple(dest.values()),(adapter.home,))
    if errors: raise ValueError('\n'.join(errors))
    # Re-scan backup because mtimes/file identities differ from the original source.
    inv=scan_roots(src,True)
    operation_id=uuid.uuid4().hex
    journal.begin(operation_id,{'kind':'import','resources':['bundle:'+m['bundle_id']], 'bundle':str(bundle),'destinations':dest})
    try:
        journal.record(operation_id,'copying',{})
        require(copy_verified(inv,src,dest,progress,cancelled))
        journal.record(operation_id,'verified',{})
        pid=adapter.create_project(m['project']['name'],tuple(dest.values()),'import-'+m['bundle_id'])
        journal.record(operation_id,'relinking',{'project_id':pid})
        mapping={clean_path(r['original_path']):dest[r['id']] for r in m['roots']}
        for t in m['conversations']:
            if cancelled(): raise ValueError('가져오기를 취소했습니다. 이미 등록한 항목은 작업 기록에서 확인하세요.')
            cwd=map_path(clean_path(t['cwd']),mapping)
            runtime=tuple(map_path(clean_path(p),mapping) for p in t.get('runtime_roots',[])) or (cwd,)
            adapter.import_conversation(safe_child(bundle,t['rollout']),t['id'],cwd,runtime,pid)
            if t.get('title'): adapter.call('thread/name/set',{'threadId':t['id'],'name':t['title']})
            if t.get('archived'): adapter.call('thread/archive',{'threadId':t['id']})
            read=adapter.read_thread(t['id'],True)
            if clean_path(read['cwd']).resolve()!=cwd.resolve() or read.get('projectId')!=pid: raise ValueError('가져온 대화의 연결 검증에 실패했습니다.')
        p=Project(pid,m['project']['name'],tuple(dest.values()))
        adapter.sync_desktop_state([p],{t['id']:pid for t in m['conversations']})
        journal.record(operation_id,'completed',{})
        return OperationResult('completed','복원 검증 완료','대화 등록 완료','백업 유지','모바일 미확인')
    except Exception as exc:
        journal.record(operation_id,'needs_recovery',{'error':str(exc)})
        return OperationResult('needs_recovery','부분 복원','등록 확인 필요','백업 유지','모바일 미확인',(str(exc),))
