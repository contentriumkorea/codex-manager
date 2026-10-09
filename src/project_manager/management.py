"""Verified metadata edits and recoverable deletion of explicitly selected records."""
import json
import shutil
import uuid
from dataclasses import dataclass,asdict,replace
from pathlib import Path
from .models import Project,Conversation,FileEntry,Inventory,OperationResult
from .operations import change_connections,require
from .bundles import write_json
from .files import digest,scan_roots,copy_verified,validate_paths,safe_child
from .cleanup import remove_verified_files


def valid_name(name):
    name=name.strip()
    if not name or len(name)>256 or any(ord(c)<32 for c in name):
        raise ValueError('이름은 줄바꿈 없이 1~256자로 입력하세요.')
    return name


def fresh_threads(adapter,threads):
    snapshot=adapter.snapshot();current={t.id:t for t in snapshot.conversations}
    if any(not same_thread(current.get(t.id),t) for t in threads):raise ValueError('선택한 대화가 바뀌었습니다. 새로고침 후 다시 선택하세요.')
    if any(t.running for t in threads):raise ValueError('실행 중인 대화는 변경할 수 없습니다.')
    return snapshot


def same_thread(current,expected):
    if current is None:return False
    # The fast UI catalog omits runtime roots and attachments. Compare its
    # persisted identity/revision, then use the full snapshot for all writes.
    if expected.runtime_roots and current.runtime_roots!=expected.runtime_roots:return False
    if expected.attachments and current.attachments!=expected.attachments:return False
    return replace(current,runtime_roots=expected.runtime_roots,attachments=expected.attachments)==expected


def rename_project(adapter,project,name,journal):
    adapter.ensure_write_allowed();name=valid_name(name)
    if next((p for p in adapter.snapshot().projects if p.id==project.id),None)!=project:
        raise ValueError('프로젝트가 바뀌었습니다. 새로고침하세요.')
    return change_connections(adapter,replace(project,name=name),{},journal)


def set_thread_name(adapter,tid,name):
    adapter.call('thread/name/set',{'threadId':tid,'name':name})
    if next(t for t in adapter.snapshot().conversations if t.id==tid).title!=name:
        raise ValueError('대화 제목 변경이 저장되지 않았습니다.')


def rename_thread(adapter,thread,name,journal):
    adapter.ensure_write_allowed();name=valid_name(name);fresh_threads(adapter,(thread,))
    op=uuid.uuid4().hex
    journal.begin(op,{'kind':'rename-thread','home':str(adapter.home),'resources':['thread:'+thread.id]+([thread.project_id] if thread.project_id else []),
                      'thread_id':thread.id,'old_name':thread.title,'new_name':name})
    try:
        set_thread_name(adapter,thread.id,name)
        journal.record(op,'completed',{})
        return OperationResult('completed','파일 유지','대화 제목 변경 완료','대화 기록 유지')
    except Exception as exc:
        journal.record(op,'needs_recovery',{'error':str(exc)})
        return OperationResult('needs_recovery','파일 유지','제목 변경 확인 필요','대화 기록 유지',errors=(str(exc),))


def move_threads(adapter,threads,target_id,journal):
    adapter.ensure_write_allowed();snapshot=fresh_threads(adapter,threads)
    target=next((p for p in snapshot.projects if p.id==target_id),None)
    if not threads or not target:raise ValueError('대화와 대상 프로젝트를 선택하세요.')
    return change_connections(adapter,target,{t.id:target.id for t in threads},journal)


@dataclass(frozen=True)
class DeletePlan:
    project: Project|None
    threads: tuple[Conversation,...]
    requested: tuple[str,...]
    inferred: tuple[str,...]=()


def plan_delete(snapshot,project_id=None,thread_ids=(),inferred=()):
    project=next((p for p in snapshot.projects if p.id==project_id),None) if project_id else None
    if project_id and not project:raise ValueError('프로젝트가 없습니다.')
    ids={t.id for t in snapshot.conversations if t.project_id==project_id} if project else set(thread_ids)
    if project:ids.update(inferred)
    if not project and not ids:raise ValueError('삭제할 대화를 선택하세요.')
    requested=tuple(sorted(ids));known={t.id for t in snapshot.conversations}
    if not ids<=known:raise ValueError('선택한 대화가 바뀌었습니다.')
    while True:
        children={t.id for t in snapshot.conversations if t.parent_id in ids}
        if children<=ids:break
        ids.update(children)
    threads=tuple(sorted((t for t in snapshot.conversations if t.id in ids),key=lambda t:t.id))
    if any(t.running for t in threads):raise ValueError('실행 중인 대화가 있습니다. 작업 종료 후 삭제하세요.')
    if project and any(t.project_id not in (None,project.id) for t in threads):
        raise ValueError('다른 프로젝트의 자식 대화가 있습니다. 대화별로 범위를 확인하세요.')
    if any(t.attachments for t in threads):raise ValueError('외부 첨부가 연결된 대화는 안전한 복원이 확인되지 않아 삭제할 수 없습니다.')
    return DeletePlan(project,threads,requested,tuple(inferred))


def deletion_current(adapter,plan):
    snapshot=adapter.snapshot()
    fresh=plan_delete(snapshot,plan.project.id if plan.project else None,plan.requested,plan.inferred)
    if fresh.project!=plan.project or len(fresh.threads)!=len(plan.threads) or any(not same_thread(a,b) for a,b in zip(fresh.threads,plan.threads)):
        raise ValueError('삭제할 프로젝트나 대화가 바뀌었습니다. 목록을 다시 확인하세요.')
    return snapshot


def check_file_ownership(snapshot,plan,roots):
    selected={t.id for t in plan.threads}
    overlap=lambda a,b:a.resolve().is_relative_to(b.resolve()) or b.resolve().is_relative_to(a.resolve())
    for p in snapshot.projects:
        if p.id!=plan.project.id and any(overlap(a,b) for a in p.roots for b in roots):
            raise ValueError('다른 프로젝트와 공유하는 폴더입니다: '+p.name)
    for t in snapshot.conversations:
        if t.id not in selected and any(overlap(a,b) for a in (t.cwd,*t.runtime_roots) for b in roots):
            raise ValueError('다른 대화가 사용하는 폴더입니다: '+t.title)


def delete_items(adapter,plan,journal,recovery_root,delete_files=False,progress=lambda *_:None,cancelled=lambda:False):
    adapter.ensure_write_allowed();snapshot=deletion_current(adapter,plan)
    plan=plan_delete(snapshot,plan.project.id if plan.project else None,plan.requested,plan.inferred)
    if cancelled():raise ValueError('삭제를 취소했습니다.')
    if delete_files and not plan.project:raise ValueError('파일 삭제는 프로젝트에서만 선택하세요.')
    roots={f'root-{i+1:02d}':p for i,p in enumerate(plan.project.roots)} if delete_files else {}
    if roots:
        check_file_ownership(snapshot,plan,tuple(roots.values()))
        errors=validate_paths(tuple(roots.values()),(),(adapter.home,journal.path.parent))
        if errors:raise ValueError('\n'.join(errors))
    op=uuid.uuid4().hex;recovery=Path(recovery_root).resolve()/op
    errors=validate_paths(tuple(roots.values()),(recovery,),(adapter.home,))
    if errors:raise ValueError('\n'.join(errors))
    inv=scan_roots(roots,True)
    if inv.blockers:raise ValueError('\n'.join(inv.blockers))
    recovery.mkdir(parents=True)
    hashes={}
    # Finish and verify recovery material before the first destructive RPC.
    for t in plan.threads:
        uuid.UUID(t.id)
        if not t.rollout or not t.rollout.is_file():raise ValueError('대화 원본을 찾을 수 없습니다: '+t.title)
        saved=recovery/(t.id+'.jsonl');before=digest(t.rollout);shutil.copyfile(t.rollout,saved)
        if digest(saved)!=before or digest(t.rollout)!=before:raise ValueError('백업 중 대화가 바뀌었습니다.')
        hashes[t.id]=before
    dest={rid:recovery/'files'/rid for rid in roots}
    require(copy_verified(inv,roots,dest,progress,cancelled))
    data={'project':asdict(plan.project) if plan.project else None,'threads':[asdict(t) for t in plan.threads],
          'hashes':hashes,'roots':roots,'inventory':asdict(inv)}
    write_json(recovery/'metadata.json',data)
    snapshot=deletion_current(adapter,plan)
    if roots:check_file_ownership(snapshot,plan,tuple(roots.values()))
    if cancelled():raise ValueError('삭제를 취소했습니다. 원본은 유지됩니다.')
    resources=list({'thread:'+t.id for t in plan.threads}|{t.project_id for t in plan.threads if t.project_id}|({plan.project.id} if plan.project else set()))
    journal.begin(op,{'kind':'management-delete','home':str(adapter.home),'resources':resources,
                      'recovery':str(recovery),'metadata_digest':digest(recovery/'metadata.json'),
                      'label':plan.project.name if plan.project else f'대화 {len(plan.threads)}개'})
    try:
        ids={t.id for t in plan.threads}
        for t in plan.threads:
            if t.parent_id not in ids:
                journal.record(op,'deleting',{'thread_id':t.id});adapter.delete_thread(t.id)
        if any(t.id in ids for t in adapter.snapshot().conversations):raise ValueError('삭제한 대화가 목록에 남아 있습니다.')
        if roots:require(remove_verified_files(inv,roots,dest))
        if plan.project:
            adapter.delete_project(plan.project.id)
            if adapter.project_exists(plan.project.id):raise ValueError('삭제한 프로젝트가 목록에 남아 있습니다.')
        adapter.sync_desktop_state([],{tid:None for tid in ids},[plan.project.id] if plan.project else [],deleted=ids)
        journal.record(op,'completed',{})
        return OperationResult('completed','작업 파일 삭제 완료' if delete_files else '작업 파일 유지',
                               '프로젝트·대화 삭제 완료' if plan.project else '대화 삭제 완료','복구 사본: '+str(recovery))
    except Exception as exc:
        journal.record(op,'needs_recovery',{'error':str(exc)})
        return OperationResult('needs_recovery','복구 사본 유지','삭제 일부 완료','작업 복구에서 복원하세요.',errors=(str(exc),))


def recover_management(operation,adapter,journal):
    adapter.ensure_write_allowed();payload=operation['payload'];op=operation['id']
    if Path(payload['home']).resolve()!=adapter.home.resolve():raise ValueError('작업을 시작한 Codex 저장소를 선택하세요.')
    if any(e['payload'].get('recovered') for e in journal.events(op)):
        return OperationResult('completed','복구 확인 완료','이미 복구한 작업입니다.','파일 유지')
    if payload['kind']=='rename-thread':
        current=next(t for t in adapter.snapshot().conversations if t.id==payload['thread_id'])
        if current.title not in (payload['old_name'],payload['new_name']):raise ValueError('대화 제목이 이후에 바뀌었습니다.')
        set_thread_name(adapter,current.id,payload['old_name'])
    else:
        recovery=Path(payload['recovery'])
        if digest(recovery/'metadata.json')!=payload['metadata_digest']:raise ValueError('복구 정보가 변경됐습니다.')
        data=json.loads((recovery/'metadata.json').read_text(encoding='utf-8'))
        for t in data['threads']:
            if digest(safe_child(recovery,t['id']+'.jsonl'))!=data['hashes'][t['id']]:raise ValueError('복구 사본 검증 실패')
        roots={rid:Path(p) for rid,p in data['roots'].items()}
        saved=data['inventory'];inv=Inventory(tuple(FileEntry(**e) for e in saved['entries']),(),saved['logical_bytes'],None,tuple(tuple(d) for d in saved['directories']))
        require(copy_verified(inv,{rid:recovery/'files'/rid for rid in roots},roots,lambda *_:None,lambda:False))
        old=data['project'];projects=[];pid=None
        if old:
            previous=next((p for p in adapter.snapshot().projects if p.id==old['id']),None) if adapter.project_exists(old['id']) else None
            if previous and (previous.name!=old['name'] or previous.roots!=tuple(Path(x) for x in old['roots'])):
                raise ValueError('프로젝트가 이후에 변경돼 덮어쓸 수 없습니다.')
            pid=old['id'] if previous else adapter.create_project(old['name'],tuple(Path(x) for x in old['roots']),'restore-delete-'+op)
            actual=adapter.call('project/read',{'projectId':pid})['project']
            from .catalog import clean_path
            if actual['name']!=old['name'] or tuple(clean_path(r['path']).resolve() for r in actual['roots'])!=tuple(Path(r).resolve() for r in old['roots']):
                raise ValueError('복원 중 만들어진 프로젝트가 변경됐습니다. 현재 설정을 덮어쓰지 않습니다.')
            projects=[Project(pid,old['name'],tuple(Path(x) for x in old['roots']))]
        known={t.id:t for t in adapter.snapshot().conversations};assignments={}
        from .codex.portability import transcript
        restoring={e['payload']['thread_id'] for e in journal.events(op) if e['phase']=='restoring-thread'}
        for t in data['threads']:
            target=pid if old and t['project_id']==old['id'] else t['project_id']
            if target and not adapter.project_exists(target):raise ValueError('복원할 대화의 프로젝트가 없습니다.')
            if t['id'] in known:
                current=known[t['id']]
                actual_membership=adapter.read_thread(t['id']).get('projectId')
                baseline=list(transcript(recovery/(t['id']+'.jsonl')))
                current_history=list(transcript(current.rollout)) if current.rollout else None
                if (current.cwd!=Path(t['cwd']) or current.runtime_roots!=tuple(Path(x) for x in t['runtime_roots']) or
                    current.parent_id!=t['parent_id'] or current_history is None or current_history[:len(baseline)]!=baseline or
                    actual_membership not in ((None,target) if t['id'] in restoring else (target,)) or
                    (t['id'] not in restoring and current.title!=t['title'])):
                    raise ValueError('동일한 대화가 이미 있습니다. 현재 기록은 덮어쓰지 않습니다.')
            else:
                journal.record(op,'restoring-thread',{'thread_id':t['id']})
                adapter.import_conversation(recovery/(t['id']+'.jsonl'),t['id'],Path(t['cwd']),tuple(Path(x) for x in t['runtime_roots']),target)
            if t['id'] not in known or t['id'] in restoring:
                adapter.assign_conversation(t['id'],target)
                if t['title']:set_thread_name(adapter,t['id'],t['title'])
                if t['archived']:adapter.call('thread/archive',{'threadId':t['id']})
            assignments[t['id']]=target
        adapter.sync_desktop_state(projects,assignments,[old['id']] if old and pid!=old['id'] else [])
    journal.record(op,'completed',{'recovered':True})
    return OperationResult('completed','파일 복구 확인 완료','이전 기록 복구 완료','복구 사본 유지')
