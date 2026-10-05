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
    while True:
        children=[t for t in snapshot.conversations if t.parent_id in selected and t.id not in selected]
        if not children: break
        if any(t.project_id not in (None,source.id) for t in children):
            raise ValueError('다른 프로젝트의 자식 대화가 연결돼 있습니다. 함께 처리할 범위를 먼저 확인하세요.')
        conversations.extend(children);selected.update(t.id for t in children)
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
    operation_id=uuid.uuid4().hex
    recovery=Path(recovery_root)/operation_id;recovery.mkdir(parents=True)
    journal.begin(operation_id,{'kind':'merge' if target else 'move','resources':[source.id]+([target.id] if target else []),
                                'source':asdict(source),'destinations':destinations,'recovery':str(recovery)})
    moved=[];project_changed=False;linked_done=False
    try:
        for t in conversations:
            if not t.rollout or not t.rollout.exists(): raise ValueError(f'대화 원본이 없습니다: {t.title}')
            saved=recovery/f'{t.id}.jsonl';shutil.copyfile(t.rollout,saved)
            if digest(saved)!=digest(t.rollout): raise ValueError('복구 사본 검증 실패')
        write_json(recovery/'metadata.json',{'project':asdict(source),'target':asdict(target) if target else None,'conversations':[asdict(t) for t in conversations],'destinations':destinations,'inventory':[asdict(e) for e in inventory.entries],'directories':inventory.directories})
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
            moved.append(t)
            adapter.relocate_conversation(t.id,map_path(t.cwd,mapping),tuple(map_path(r,mapping) for r in t.runtime_roots) or (map_path(t.cwd,mapping),))
            adapter.assign_conversation(t.id,new_project.id)
        journal.record(operation_id,'linked',{})
        linked_done=True
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
        for t in reversed(moved) if not linked_done else []:
            try:
                adapter.relocate_conversation(t.id,t.cwd,t.runtime_roots or (t.cwd,));adapter.assign_conversation(t.id,source.id)
            except Exception as error: rollback_errors.append(str(error))
        if project_changed and not linked_done:
            try: adapter.update_project(source)
            except Exception as error: rollback_errors.append(str(error))
        journal.record(operation_id,'needs_recovery',{'error':str(exc),'rollback_errors':rollback_errors})
        return OperationResult('needs_recovery','복사본 유지','복구 확인 필요','원본 파일 유지','모바일 미확인',(str(exc),*rollback_errors))


def import_bundle(bundle, destinations, adapter, journal, progress=lambda *_:None, cancelled=lambda:False, resume_id=None):
    adapter.ensure_write_allowed();require(verify_bundle(bundle));m=load_manifest(bundle)
    if m['dependencies']: raise ValueError('외부 첨부·자식 대화·worktree 의존성을 해결한 뒤 가져올 수 있습니다.')
    snapshot=adapter.snapshot();existing={t.id:t for t in snapshot.conversations}
    previous_pid=None
    if resume_id:
        previous_pid=next((e['payload'].get('project_id') for e in reversed(journal.events(resume_id)) if e['payload'].get('project_id')),None)
    conflicts=[t['id'] for t in m['conversations'] if t['id'] in existing and (not resume_id or existing[t['id']].project_id!=previous_pid)]
    if conflicts: raise ValueError('대상 컴퓨터에 같은 ID의 대화가 있습니다. 기존 대화를 덮어쓰지 않습니다: '+', '.join(conflicts))
    src={r['id']:safe_child(bundle,r['bundle_path']) for r in m['roots']}
    dest={rid:Path(destinations[rid]).resolve() for rid in src}
    errors=validate_paths(tuple(src.values()),tuple(dest.values()),(adapter.home,))
    if errors: raise ValueError('\n'.join(errors))
    # Re-scan backup because mtimes/file identities differ from the original source.
    inv=scan_roots(src,True)
    operation_id=resume_id or uuid.uuid4().hex
    if not resume_id: journal.begin(operation_id,{'kind':'import','resources':['bundle:'+m['bundle_id']], 'bundle':str(bundle),'destinations':dest})
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
            if t['id'] not in existing:
                adapter.import_conversation(safe_child(bundle,t['rollout']),t['id'],cwd,runtime,pid)
            else:
                from .codex.portability import transcript
                baseline=list(transcript(safe_child(bundle,t['rollout'])))
                current=list(transcript(existing[t['id']].rollout))
                if current[:len(baseline)]!=baseline or existing[t['id']].cwd.resolve()!=cwd.resolve():
                    raise ValueError('이미 등록된 대화가 복구 계획과 다릅니다.')
            if t.get('title'): adapter.call('thread/name/set',{'threadId':t['id'],'name':t['title']})
            if t.get('archived'): adapter.call('thread/archive',{'threadId':t['id']})
            read=adapter.read_thread(t['id'],True)
            if clean_path(read['cwd']).resolve()!=cwd.resolve() or read.get('projectId')!=pid: raise ValueError('가져온 대화의 연결 검증에 실패했습니다.')
        p=Project(pid,m['project']['name'],tuple(dest.values()))
        adapter.sync_desktop_state([p],{t['id']:pid for t in m['conversations']})
        reports=adapter.home/'project-manager-import-reports';reports.mkdir(exist_ok=True)
        restored=adapter.snapshot()
        write_json(reports/(m['bundle_id']+'.json'),{'bundle_id':m['bundle_id'],'project_id':pid,
                   'destinations':dest,'threads':{t['id']:{'offset':next(x for x in restored.conversations if x.id==t['id']).rollout.stat().st_size} for t in m['conversations']}})
        journal.record(operation_id,'completed',{})
        return OperationResult('completed','복원 검증 완료','대화 등록 완료','백업 유지','모바일 미확인')
    except Exception as exc:
        journal.record(operation_id,'needs_recovery',{'error':str(exc)})
        return OperationResult('needs_recovery','부분 복원','등록 확인 필요','백업 유지','모바일 미확인',(str(exc),))


def recover_operation(operation_id,adapter,journal,progress=lambda *_:None):
    adapter.ensure_write_allowed()
    operation=next(p for p in journal.pending() if p['id']==operation_id)
    payload=operation['payload']
    if payload['kind']=='import':
        return import_bundle(Path(payload['bundle']),{k:Path(v) for k,v in payload['destinations'].items()},adapter,journal,progress,resume_id=operation_id)
    if payload['kind'] not in ('move','merge'): raise ValueError('이 작업은 검증된 백업에서 가져오기로 복구하세요.')
    recovery=Path(payload['recovery'])
    metadata=json.loads((recovery/'metadata.json').read_text(encoding='utf-8'))
    old=metadata['project'];roots=tuple(clean_path(p) for p in old['roots'])
    source_map={f'root-{i+1:02d}':clean_path(metadata['destinations'][str(root)]) for i,root in enumerate(roots)}
    original_map={f'root-{i+1:02d}':root for i,root in enumerate(roots)}
    # Restore only the files captured in this operation; keep unrelated new files.
    captured=metadata['inventory'];entries=[]
    for e in captured:
        p=safe_child(source_map[e['root_id']],e['relative_path'])
        if not p.exists():
            original=safe_child(original_map[e['root_id']],e['relative_path'])
            if original.exists() and digest(original)==e['sha256']: continue
            raise ValueError('복구에 필요한 파일이 없습니다: '+str(p))
        if digest(p)!=e['sha256']: raise ValueError('복사본이 변경돼 자동 복구할 수 없습니다: '+str(p))
        entries.append(FileEntry(e['root_id'],e['relative_path'],e['size'],e['sha256'],e['file_id'],p.stat().st_mtime_ns))
    inv=Inventory(tuple(entries),(),sum(e.size for e in entries),None,tuple(tuple(d) for d in metadata['directories']))
    require(copy_verified(inv,source_map,original_map,progress,lambda:False))
    snapshot=adapter.snapshot();pid=old['id']
    if not any(p.id==pid for p in snapshot.projects):
        pid=adapter.create_project(old['name'],roots,'recover-'+operation_id)
    original=Project(pid,old['name'],roots,tuple(old.get('legacy_ids',[])))
    adapter.update_project(original)
    known={t.id:t for t in adapter.snapshot().conversations}
    for t in metadata['conversations']:
        cwd=clean_path(t['cwd']);runtime=tuple(clean_path(p) for p in t['runtime_roots']) or (cwd,)
        if t['id'] in known: adapter.relocate_conversation(t['id'],cwd,runtime)
        else: adapter.import_conversation(recovery/(t['id']+'.jsonl'),t['id'],cwd,runtime,pid)
        adapter.assign_conversation(t['id'],pid)
    adapter.sync_desktop_state([original],{t['id']:pid for t in metadata['conversations']})
    journal.record(operation_id,'completed',{'recovered':True})
    return OperationResult('completed','원본 위치 복구 완료','이전 연결 복구 완료','복사본 유지','모바일 미확인')
