"""Restore only missing items after interrupted, verified source cleanup."""
import json
from pathlib import Path
from .bundles import load_manifest,verify_bundle
from .files import digest,safe_child,validate_paths,copy_verified,inside
from .models import Project,FileEntry,Inventory,OperationResult
from .codex.portability import prepare_rollout


def prefix_matches(source,target,logical=False):
    with source.open('rb') as original,target.open('rb') as current:
        if logical:
            for line in original:
                other=current.readline()
                if not other or json.loads(line)!=json.loads(other): return False
        else:
            while chunk:=original.read(1024*1024):
                if current.read(len(chunk))!=chunk: return False
    return True


def recover_cleanup(operation,bundle,adapter,journal,progress):
    payload=operation['payload'];op=operation['id'];check=verify_bundle(bundle)
    if not check.ok: raise ValueError('\n'.join(check.errors))
    m=load_manifest(bundle)
    if payload.get('bundle_id')!=m['bundle_id'] or payload.get('manifest_digest')!=digest(bundle/'manifest.json'):
        raise ValueError('중단된 원본 정리와 다른 백업입니다. 원래 백업을 선택하세요.')
    old=payload['project'];roots=tuple(Path(x) for x in old['roots'])
    if m['project']['id']!=old['id'] or roots!=tuple(Path(x) for x in m['project']['roots']) or m['dependencies']:
        raise ValueError('원래 정리 범위와 다른 백업입니다.')
    src={r['id']:safe_child(bundle,r['bundle_path']) for r in m['roots']}
    dest={r['id']:Path(r['original_path']) for r in m['roots']}
    errors=validate_paths(tuple(src.values()),tuple(dest.values()),(adapter.home,))
    if errors: raise ValueError('\n'.join(errors))
    events=journal.events(op)
    pid=next((e['payload']['restore_project_id'] for e in reversed(events) if 'restore_project_id' in e['payload']),old['id'])
    intents={e['payload']['thread_id']:e['payload'] for e in events if e['phase']=='restore-thread-intent'}
    snapshot=adapter.snapshot();project=next((p for p in snapshot.projects if p.id==pid),None)
    if project and hasattr(adapter,'project_exists') and not adapter.project_exists(pid): project=None
    if not project and any(e['phase']=='restore-project-intent' for e in events):
        # Retry the exact idempotent key before treating its unknown-result project
        # as an unrelated project sharing these folders.
        pid=adapter.create_project(old['name'],roots,'cleanup-recover-'+op)
        journal.record(op,'restoring',{'restore_project_id':pid})
        snapshot=adapter.snapshot();project=next((p for p in snapshot.projects if p.id==pid),None)
    if project and tuple(p.resolve() for p in project.roots)!=tuple(p.resolve() for p in roots):
        raise ValueError('프로젝트 폴더 연결이 바뀌어 자동 복구할 수 없습니다.')
    for other in snapshot.projects:
        if other.id not in (pid,old['id']) and any(inside(r.resolve(),s.resolve()) or inside(s.resolve(),r.resolve()) for r in other.roots for s in roots):
            raise ValueError('복구 위치를 다른 프로젝트가 사용하고 있습니다: '+other.name)
    known={t.id:t for t in snapshot.conversations};preserved={};records={t['id']:t for t in m['conversations']}
    for tid,t in records.items():
        cwd=Path(t['cwd']);runtime=tuple(Path(x) for x in t['runtime_roots'])
        if not any(inside(cwd.resolve(),r.resolve()) for r in roots) or any(not any(inside(x.resolve(),r.resolve()) for r in roots) for x in runtime):
            raise ValueError('백업의 대화 경로가 원래 프로젝트 밖에 있습니다.')
        if tid not in known: continue
        current=known[tid];intent=intents.get(tid)
        expected_pid=pid if t['project_id']==old['id'] else t['project_id']
        allowed=(None,expected_pid) if intent else (expected_pid,)
        expected=safe_child(bundle,t['rollout'])
        if intent:
            expected=safe_child(journal.path.parent,intent['expected_relative'])
            if digest(expected)!=intent['expected_sha256']: raise ValueError('복구 기록의 대화 사본이 바뀌었습니다.')
        expected_runtime=runtime or (cwd,) if intent else runtime
        membership=adapter.read_thread(tid).get('projectId') if intent else current.project_id
        if (membership not in allowed or current.parent_id!=t['parent_id'] or current.cwd.resolve()!=cwd.resolve()
            or tuple(p.resolve() for p in current.runtime_roots)!=tuple(p.resolve() for p in expected_runtime)
            or not current.rollout or not prefix_matches(expected,current.rollout,logical=bool(intent))):
            raise ValueError('남아 있는 대화가 원래 기록과 다릅니다. 기존 대화를 덮어쓰지 않습니다: '+tid)
        if not intent: preserved[tid]=(current,digest(current.rollout))
    # Parent-first registration. Reject cycles before any changes.
    ordered=[];waiting=dict(records)
    while waiting:
        ready=[tid for tid,t in waiting.items() if t['parent_id'] not in waiting]
        if not ready: raise ValueError('대화의 부모 연결에 순환이 있습니다.')
        for tid in ready: ordered.append(waiting.pop(tid))
    try:
        entries=[];changed=[]
        for e in m['inventory']:
            source=safe_child(src[e['root_id']],e['relative_path']);target=safe_child(dest[e['root_id']],e['relative_path'])
            if target.exists():
                if not target.is_file() or digest(target)!=e['sha256']: changed.append(str(target))
            else: entries.append(FileEntry(e['root_id'],e['relative_path'],e['size'],e['sha256'],e['file_id'],source.stat().st_mtime_ns))
        inventory=Inventory(tuple(entries),(),sum(e.size for e in entries),None,tuple(tuple(d) for d in m['directories']))
        check=copy_verified(inventory,src,dest,progress,lambda:False)
        if not check.ok: raise ValueError('\n'.join(check.errors))
        if not project:
            journal.record(op,'restore-project-intent',{'key':'cleanup-recover-'+op})
            pid=adapter.create_project(old['name'],roots,'cleanup-recover-'+op)
            journal.record(op,'restoring',{'restore_project_id':pid})
            project=Project(pid,old['name'],roots)
        for t in ordered:
            tid=t['id'];cwd=Path(t['cwd']);runtime=tuple(Path(x) for x in t['runtime_roots']) or (cwd,)
            desired=pid if t['project_id']==old['id'] else t['project_id']
            if tid in preserved: continue
            if tid not in known:
                recovery=safe_child(journal.path.parent,'cleanup-recovery/'+op);recovery.mkdir(parents=True,exist_ok=True)
                expected=prepare_rollout(safe_child(bundle,t['rollout']),recovery,cwd,runtime)
                journal.record(op,'restore-thread-intent',{'thread_id':tid,'expected_relative':expected.relative_to(journal.path.parent).as_posix(),'expected_sha256':digest(expected)})
                adapter.import_conversation(expected,tid,cwd,runtime,desired)
            adapter.assign_conversation(tid,desired)
            if t.get('title'): adapter.call('thread/name/set',{'threadId':tid,'name':t['title']})
            if t.get('archived'): adapter.call('thread/archive',{'threadId':tid})
        final=adapter.snapshot();final_threads={t.id:t for t in final.conversations}
        for t in ordered:
            actual=final_threads.get(t['id']);expected_pid=pid if t['project_id']==old['id'] else t['project_id']
            runtime=tuple(Path(x).resolve() for x in t['runtime_roots']) or (Path(t['cwd']).resolve(),)
            if not actual or actual.project_id!=expected_pid or actual.cwd.resolve()!=Path(t['cwd']).resolve() or actual.parent_id!=t['parent_id']:
                raise ValueError('복구한 대화의 연결 확인에 실패했습니다.')
            if t['id'] not in preserved and tuple(p.resolve() for p in actual.runtime_roots)!=runtime:
                raise ValueError('복구한 대화의 작업 폴더가 다릅니다.')
            if t['id'] not in preserved and (actual.archived!=t['archived'] or actual.title!=t['title']):
                raise ValueError('복구한 대화의 제목·보관 상태가 다릅니다.')
        for tid,(original,sha) in preserved.items():
            if final_threads[tid]!=original or digest(original.rollout)!=sha: raise ValueError('복구 도중 남아 있는 대화가 변경됐습니다.')
        for e in m['inventory']:
            target=safe_child(dest[e['root_id']],e['relative_path'])
            if str(target) not in changed and (not target.is_file() or digest(target)!=e['sha256']): raise ValueError('복구 파일 재검증 실패: '+str(target))
        adapter.sync_desktop_state([project],{t['id']:pid if t['project_id']==old['id'] else t['project_id'] for t in ordered},[old['id']] if pid!=old['id'] else [])
        journal.record(op,'completed',{'recovered':True,'preserved_changed_files':changed})
        return OperationResult('completed','누락 파일 복구 완료'+(' · 변경 파일 보존 '+str(len(changed))+'개' if changed else ''),'누락 대화 복구 완료','백업·기존 파일 유지','모바일 미확인',report_id=op)
    except Exception as exc:
        journal.record(op,'needs_recovery',{'error':str(exc)})
        return OperationResult('needs_recovery','백업·기존 파일 유지','누락 항목 복구 진행 중','다시 복구할 수 있습니다.','모바일 미확인',(str(exc),),report_id=op)
