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


def change_connections(adapter,project,assignments,journal):
    adapter.ensure_write_allowed();snapshot=adapter.snapshot()
    original=next(p for p in snapshot.projects if p.id==project.id)
    threads={t.id:t for t in snapshot.conversations}
    if any(tid not in threads for tid in assignments): raise ValueError('선택한 대화가 바뀌었습니다.')
    known_projects={p.id for p in snapshot.projects}
    if any(pid is not None and pid not in known_projects for pid in assignments.values()): raise ValueError('대상 프로젝트가 없습니다.')
    old={tid:threads[tid].project_id for tid in assignments};operation_id=uuid.uuid4().hex
    journal.begin(operation_id,{'kind':'connections','home':str(adapter.home),'resources':list({project.id,*[p for p in assignments.values() if p],*[p for p in old.values() if p]}),
                                'project':asdict(original),'assignments':old,'expected_project':asdict(project),'expected_assignments':assignments})
    try:
        journal.record(operation_id,'relinking',{})
        adapter.update_project(project)
        for tid,pid in assignments.items(): adapter.assign_conversation(tid,pid)
        adapter.sync_desktop_state([project],assignments)
        journal.record(operation_id,'completed',{})
        return OperationResult('completed','파일 유지','연결 변경 완료','원본 파일 유지',report_id=operation_id)
    except Exception as exc:
        errors=[]
        try:
            adapter.update_project(original)
            for tid,pid in old.items(): adapter.assign_conversation(tid,pid)
            adapter.sync_desktop_state([original],old)
        except Exception as rollback: errors.append(str(rollback))
        journal.record(operation_id,'needs_recovery',{'error':str(exc),'rollback_errors':errors})
        return OperationResult('needs_recovery','파일 유지','이전 연결 복구 확인 필요','원본 파일 유지',errors=(str(exc),*errors),report_id=operation_id)


def transfer_project(adapter, project, destinations, journal, recovery_root, target=None, clean=False, progress=lambda *_:None, cancelled=lambda:False):
    adapter.ensure_write_allowed()
    snapshot=adapter.snapshot()
    source=next(p for p in snapshot.projects if p.id==project.id)
    if target: target=next(p for p in snapshot.projects if p.id==target.id)
    conversations=[t for t in snapshot.conversations if t.project_id==source.id]
    selected={t.id for t in conversations}
    while True:
        children=[t for t in snapshot.conversations if t.parent_id in selected and t.id not in selected]
        if not children: break
        if any(t.project_id not in (None,source.id) for t in children):
            raise ValueError('다른 프로젝트의 자식 대화가 연결돼 있습니다. 함께 처리할 범위를 먼저 확인하세요.')
        conversations.extend(children);selected.update(t.id for t in children)
    from .grouping import display_membership
    from .settings import read_settings
    inferred=display_membership(snapshot,read_settings(adapter.home/'.codex-global-state.json')).inferred
    if any(pid==source.id and tid not in selected for tid,pid in inferred.items()):
        raise ValueError('폴더 기준 대화의 연결이 아직 확정되지 않았습니다. 새로고침하고 연결을 확정한 뒤 이동하세요.')
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
    journal.begin(operation_id,{'kind':'merge' if target else 'move','home':str(adapter.home),'resources':[source.id]+([target.id] if target else []),
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
        return OperationResult('completed','검증 완료','연결 변경 완료','원본 정리 완료' if clean else '원본 파일 유지','모바일 미확인',report_id=operation_id)
    except Exception as exc:
        rollback_errors=[]
        for t in reversed(moved) if not linked_done else []:
            try:
                adapter.relocate_conversation(t.id,t.cwd,t.runtime_roots or (t.cwd,));adapter.assign_conversation(t.id,t.project_id)
            except Exception as error: rollback_errors.append(str(error))
        if project_changed and not linked_done:
            try: adapter.update_project(target or source)
            except Exception as error: rollback_errors.append(str(error))
        if not linked_done and not rollback_errors:
            try: adapter.sync_desktop_state([source]+([target] if target else []),{t.id:t.project_id for t in conversations})
            except Exception as error: rollback_errors.append(str(error))
        journal.record(operation_id,'needs_recovery',{'error':str(exc),'rollback_errors':rollback_errors})
        return OperationResult('needs_recovery','복사본 유지','복구 확인 필요','원본 파일 유지','모바일 미확인',(str(exc),*rollback_errors),report_id=operation_id)


def import_bundle(bundle, destinations, adapter, journal, progress=lambda *_:None, cancelled=lambda:False, resume_id=None):
    adapter.ensure_write_allowed();require(verify_bundle(bundle));m=load_manifest(bundle)
    if m['dependencies']: raise ValueError('외부 첨부·자식 대화·worktree 의존성을 해결한 뒤 가져올 수 있습니다.')
    snapshot=adapter.snapshot();existing={t.id:t for t in snapshot.conversations}
    previous_pid=None
    intended=set()
    if resume_id:
        pending=next((p for p in journal.pending() if p['id']==resume_id),None)
        if not pending or pending['payload'].get('kind')!='import': raise ValueError('가져오기 복구 작업이 아닙니다.')
        plan=pending['payload']
        if Path(plan['home']).resolve()!=adapter.home.resolve(): raise ValueError('작업을 시작한 Codex 저장소를 선택하세요.')
        if plan.get('bundle_id')!=m['bundle_id'] or plan.get('manifest_digest')!=digest(bundle/'manifest.json'):
            raise ValueError('중단된 작업과 다른 백업입니다. 원래 백업 폴더를 선택하세요.')
        if {k:Path(v).resolve() for k,v in plan['destinations'].items()}!={k:Path(v).resolve() for k,v in destinations.items()}:
            raise ValueError('중단된 작업의 대상 폴더가 다릅니다.')
        previous_pid=next((e['payload'].get('project_id') for e in reversed(journal.events(resume_id)) if e['payload'].get('project_id')),None)
        intended={e['payload']['thread_id'] for e in journal.events(resume_id) if e['phase']=='registering' and 'thread_id' in e['payload']}
    conflicts=[t['id'] for t in m['conversations'] if t['id'] in existing and (not resume_id or not previous_pid or t['id'] not in intended or existing[t['id']].project_id not in (None,previous_pid))]
    if conflicts: raise ValueError('대상 컴퓨터에 같은 ID의 대화가 있습니다. 기존 대화를 덮어쓰지 않습니다: '+', '.join(conflicts))
    src={r['id']:safe_child(bundle,r['bundle_path']) for r in m['roots']}
    dest={rid:Path(destinations[rid]).resolve() for rid in src}
    errors=validate_paths(tuple(src.values()),tuple(dest.values()),(adapter.home,))
    if errors: raise ValueError('\n'.join(errors))
    # Copy exactly the verified manifest, never files added after verification.
    mapping={clean_path(r['original_path']):dest[r['id']] for r in m['roots']}
    mapped={}
    for t in m['conversations']:
        cwd=map_path(clean_path(t['cwd']),mapping)
        runtime=tuple(map_path(clean_path(p),mapping) for p in t.get('runtime_roots',[])) or (cwd,)
        mapped[t['id']]=(cwd,runtime)
    entries=[]
    for e in m['inventory']:
        path=safe_child(src[e['root_id']],e['relative_path']);stat=path.stat()
        entries.append(FileEntry(e['root_id'],e['relative_path'],e['size'],e['sha256'],str(stat.st_ino),stat.st_mtime_ns))
    inv=Inventory(tuple(entries),(),sum(e.size for e in entries),None,tuple(tuple(x) for x in m['directories']))
    operation_id=resume_id or uuid.uuid4().hex
    if not resume_id: journal.begin(operation_id,{'kind':'import','home':str(adapter.home),'resources':['bundle:'+m['bundle_id']], 'bundle':str(bundle),
                                                 'bundle_id':m['bundle_id'],'manifest_digest':digest(bundle/'manifest.json'),'destinations':dest})
    try:
        journal.record(operation_id,'copying',{})
        require(copy_verified(inv,src,dest,progress,cancelled))
        journal.record(operation_id,'verified',{})
        pid=adapter.create_project(m['project']['name'],tuple(dest.values()),'import-'+m['bundle_id'])
        journal.record(operation_id,'relinking',{'project_id':pid})
        for t in m['conversations']:
            if cancelled(): raise ValueError('가져오기를 취소했습니다. 이미 등록한 항목은 작업 기록에서 확인하세요.')
            cwd,runtime=mapped[t['id']]
            if t['id'] not in existing:
                journal.record(operation_id,'registering',{'thread_id':t['id'],'project_id':pid,'cwd':str(cwd),'runtime_roots':runtime})
                adapter.import_conversation(safe_child(bundle,t['rollout']),t['id'],cwd,runtime,pid)
            else:
                from .codex.portability import transcript
                baseline=list(transcript(safe_child(bundle,t['rollout'])))
                current=list(transcript(existing[t['id']].rollout))
                if (current[:len(baseline)]!=baseline or existing[t['id']].cwd.resolve()!=cwd.resolve() or
                    tuple(p.resolve() for p in existing[t['id']].runtime_roots)!=tuple(p.resolve() for p in runtime)):
                    raise ValueError('이미 등록된 대화가 복구 계획과 다릅니다.')
            adapter.assign_conversation(t['id'],pid)
            if t.get('title'): adapter.call('thread/name/set',{'threadId':t['id'],'name':t['title']})
            if t.get('archived'): adapter.call('thread/archive',{'threadId':t['id']})
            read=adapter.read_thread(t['id'],True)
            if clean_path(read['cwd']).resolve()!=cwd.resolve() or read.get('projectId')!=pid: raise ValueError('가져온 대화의 연결 검증에 실패했습니다.')
            active=next(x for x in adapter.snapshot().conversations if x.id==t['id'])
            if active.parent_id!=t.get('parent_id') or tuple(p.resolve() for p in active.runtime_roots)!=tuple(p.resolve() for p in runtime):
                raise ValueError('가져온 대화의 부모·작업 폴더를 확인할 수 없습니다.')
        p=Project(pid,m['project']['name'],tuple(dest.values()))
        adapter.sync_desktop_state([p],{t['id']:pid for t in m['conversations']})
        reports=adapter.home/'project-manager-import-reports';reports.mkdir(exist_ok=True)
        restored=adapter.snapshot()
        write_json(reports/(m['bundle_id']+'.json'),{'bundle_id':m['bundle_id'],'manifest_digest':digest(bundle/'manifest.json'),'project_id':pid,
                   'destinations':dest,'threads':{t['id']:{'offset':next(x for x in restored.conversations if x.id==t['id']).rollout.stat().st_size} for t in m['conversations']}})
        journal.record(operation_id,'completed',{})
        return OperationResult('completed','복원 검증 완료','대화 등록 완료','백업 유지','모바일 미확인',report_id=operation_id)
    except Exception as exc:
        journal.record(operation_id,'needs_recovery',{'error':str(exc)})
        return OperationResult('needs_recovery','부분 복원','등록 확인 필요','백업 유지','모바일 미확인',(str(exc),),report_id=operation_id)


def recover_operation(operation_id,adapter,journal,progress=lambda *_:None,bundle_override=None):
    operation=journal.operation(operation_id)
    if not operation: raise ValueError('복구할 작업을 찾을 수 없습니다.')
    if operation['payload']['kind']=='file-edit':
        if Path(operation['payload']['home']).resolve()!=adapter.home.resolve():raise ValueError('작업한 저장소를 선택하세요.')
        from .file_edits import undo_files
        return undo_files(operation,journal)
    adapter.ensure_write_allowed()
    if operation['payload']['kind'] in ('management-delete','rename-thread','rename-threads'):
        from .management import recover_management
        return recover_management(operation,adapter,journal)
    if operation['state']=='completed' and not (operation['payload']['kind']=='connections' and 'expected_project' in operation['payload']): return OperationResult('completed','복구 확인 완료','이미 완료한 작업입니다.','파일 유지',report_id=operation_id)
    payload=operation['payload']
    if Path(payload['home']).resolve()!=adapter.home.resolve(): raise ValueError('작업을 시작한 Codex 저장소를 선택하세요: '+payload['home'])
    if payload['kind']=='connections':
        if any(e['payload'].get('recovered') for e in journal.events(operation_id)):
            return OperationResult('completed','파일 유지','이미 복구했습니다.','파일 유지',report_id=operation_id)
        if 'expected_project' in payload:
            snap=adapter.snapshot();current=next((p for p in snap.projects if p.id==payload['project']['id']),None)
            encoded=json.loads(json.dumps(asdict(current),default=str)) if current else None
            if encoded not in (payload['expected_project'],payload['project']):raise ValueError('프로젝트가 이후 변경됐습니다. 현재 상태는 덮어쓰지 않습니다.')
            threads={t.id:t for t in snap.conversations}
            if any(tid not in threads or threads[tid].running or threads[tid].project_id not in (old,payload['expected_assignments'][tid]) for tid,old in payload['assignments'].items()):
                raise ValueError('대화 연결이 이후 변경됐습니다. 현재 연결은 덮어쓰지 않습니다.')
        p=payload['project'];original=Project(p['id'],p['name'],tuple(clean_path(x) for x in p['roots']),tuple(p.get('legacy_ids',[])))
        adapter.update_project(original)
        for tid,pid in payload['assignments'].items(): adapter.assign_conversation(tid,pid)
        adapter.sync_desktop_state([original],payload['assignments'])
        journal.record(operation_id,'completed',{'recovered':True})
        return OperationResult('completed','파일 유지','이전 연결 복구 완료','원본 파일 유지',report_id=operation_id)
    if payload['kind']=='import':
        return import_bundle(Path(bundle_override or payload['bundle']),{k:Path(v) for k,v in payload['destinations'].items()},adapter,journal,progress,resume_id=operation_id)
    if payload['kind']=='export-cleanup':
        from .cleanup_recovery import recover_cleanup
        return recover_cleanup(operation,Path(bundle_override or payload['bundle']),adapter,journal,progress)
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
    previous_target=metadata.get('target')
    restored_projects=[original]
    if previous_target:
        target_id=previous_target['id']
        if not any(p.id==target_id for p in adapter.snapshot().projects): raise ValueError('합칠 대상 프로젝트가 없어 자동 복구할 수 없습니다.')
        previous=Project(target_id,previous_target['name'],tuple(clean_path(p) for p in previous_target['roots']),tuple(previous_target.get('legacy_ids',[])))
        adapter.update_project(previous);restored_projects.append(previous)
    known={t.id:t for t in adapter.snapshot().conversations}
    for t in metadata['conversations']:
        cwd=clean_path(t['cwd']);runtime=tuple(clean_path(p) for p in t['runtime_roots']) or (cwd,)
        if t['id'] in known: adapter.relocate_conversation(t['id'],cwd,runtime)
        else: adapter.import_conversation(recovery/(t['id']+'.jsonl'),t['id'],cwd,runtime,pid if t['project_id']==old['id'] else t['project_id'])
        adapter.assign_conversation(t['id'],pid if t['project_id']==old['id'] else t['project_id'])
    adapter.sync_desktop_state(restored_projects,{t['id']:pid if t['project_id']==old['id'] else t['project_id'] for t in metadata['conversations']})
    journal.record(operation_id,'completed',{'recovered':True})
    return OperationResult('completed','원본 위치 복구 완료','이전 연결 복구 완료','복사본 유지','모바일 미확인',report_id=operation_id)
