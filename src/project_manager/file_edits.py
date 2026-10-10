"""Recoverable file edits below project roots; never overwrite an existing item."""
import os
import shutil
import uuid
from pathlib import Path
from .files import linked,digest
from .models import OperationResult

RECOVERY='.codex-manager-file-recovery'


def checked(path):
    path=Path(os.path.abspath(path))
    for part in (path,*path.parents):
        if part.exists() and linked(part):raise ValueError('링크 또는 클라우드 자리표시자는 자동 처리할 수 없습니다: '+str(part))
    return path


def signature(path):
    path=checked(path)
    if not path.exists():raise ValueError('파일을 찾을 수 없습니다: '+str(path))
    if path.is_file():return {'':digest(path)}
    if not path.is_dir():raise ValueError('일반 파일 또는 폴더만 처리할 수 있습니다.')
    result={'':None}
    for folder,dirs,files in os.walk(path,followlinks=False,onerror=lambda exc:(_ for _ in ()).throw(exc)):
        for name in dirs+files:
            item=checked(Path(folder)/name)
            result[item.relative_to(path).as_posix()]=None if item.is_dir() else digest(item)
    return result


def unused(path,reserved):
    stem=path.stem if path.suffix else path.name;suffix=path.suffix;n=1;candidate=path
    while candidate.exists() or str(candidate).casefold() in reserved:
        candidate=path.with_name(f'{stem} - 복사본 ({n}){suffix}');n+=1
    reserved.add(str(candidate).casefold());return candidate


def copy_item(source,target,stamp,cancelled=lambda:False):
    if target.exists():raise FileExistsError('대상에 새 항목이 생겼습니다: '+str(target))
    for relative,value in sorted(stamp.items(),key=lambda pair:(pair[0].count('/'),pair[0])):
        src=checked(source/relative if relative else source);dst=checked(target/relative if relative else target)
        if value is None:dst.mkdir()
        else:
            with src.open('rb') as inp,dst.open('xb') as out:
                while True:
                    if cancelled():raise InterruptedError('파일 작업을 취소했습니다. 원본은 유지됩니다.')
                    chunk=inp.read(1024*1024)
                    if not chunk:break
                    out.write(chunk)
            shutil.copystat(src,dst)
    if signature(source)!=stamp or signature(target)!=stamp:raise ValueError('복사 중 파일이 바뀌었습니다. 원본은 유지됩니다.')


def identity(path):
    stat=checked(path).stat();return f'{stat.st_dev}:{stat.st_ino}'


def same_device(source,destination):return source.stat().st_dev==destination.stat().st_dev


def edit_files(kind,sources,destination,roots,protected,journal,home,name=None,cancelled=lambda:False):
    if cancelled():raise InterruptedError('파일 작업을 취소했습니다.')
    if kind not in ('copy','move','delete','rename'):raise ValueError('지원하지 않는 파일 작업입니다.')
    roots=tuple(checked(p) for p in roots);protected=tuple(checked(p) for p in protected)
    home=checked(home);sources=tuple(dict.fromkeys(checked(p) for p in sources))
    if not sources or kind=='rename' and len(sources)!=1:raise ValueError('파일을 선택하세요.')
    for p in sources:
        if RECOVERY in p.parts or p==Path(p.anchor) or p.is_relative_to(home) or home.is_relative_to(p):raise ValueError('보호된 경로입니다: '+str(p))
        if kind!='copy' and (not any(p.is_relative_to(r) for r in roots) or any(q.is_relative_to(p) for q in (*roots,*protected))):
            raise ValueError('프로젝트 또는 대화의 작업 폴더는 프로젝트 관리에서 옮기세요: '+str(p))
        if any(p!=q and p.is_relative_to(q) for q in sources):raise ValueError('상위 폴더와 하위 항목을 동시에 선택할 수 없습니다.')
    destination=checked(destination)
    if kind in ('copy','move'):
        if not destination.is_dir() or not any(destination.is_relative_to(r) for r in roots) or RECOVERY in destination.parts or destination.is_relative_to(home):raise ValueError('프로젝트 안의 대상 폴더를 선택하세요.')
        if any(destination.is_relative_to(p) for p in sources):raise ValueError('폴더를 자기 자신 안에 붙여넣을 수 없습니다.')
    if kind=='rename':
        if not name or name in ('.','..') or name!=name.strip() or name.endswith('.') or any(c in name for c in '<>:/\\|?*') or any(ord(c)<32 for c in name) or RECOVERY in name:
            raise ValueError('사용할 수 없는 파일 이름입니다.')
        if sources[0].name==name:return OperationResult('completed','변경 없음','대화 유지','파일 유지')
    op=uuid.uuid4().hex;plans=[];reserved=set()
    for i,source in enumerate(sources):
        if cancelled():raise InterruptedError('파일 작업을 취소했습니다.')
        stamp=signature(source);hold=None
        root=next((r for r in roots if source.is_relative_to(r)),None)
        if kind=='delete':target=root/RECOVERY/op/(str(i)+'-'+source.name);mode='rename'
        elif kind=='rename':target=source.with_name(name);mode='rename'
        else:
            if kind=='move' and source.parent==destination:continue
            target=unused(destination/source.name,reserved);mode='copy' if kind=='copy' else 'rename'
            if kind=='move' and not same_device(source,destination):
                mode='cross_move';hold=root/RECOVERY/op/(str(i)+'-'+source.name)
        checked(target)
        if target.exists():raise FileExistsError('같은 이름의 항목이 있습니다: '+str(target))
        target_root=next((r for r in sorted(roots,key=lambda r:len(str(r)),reverse=True) if target.is_relative_to(r)),root)
        recovery=target_root/RECOVERY/op
        plans.append({'stage':str(recovery/(str(i)+'-stage')),'recovery':str(recovery),'source_id':identity(source),'source':str(source),'target':str(target),'hold':str(hold) if hold else None,'mode':mode,'signature':stamp})
    if not plans:return OperationResult('completed','같은 폴더입니다.','대화 유지','파일 유지')
    labels={'copy':'파일 복사','move':'파일 이동','delete':'파일 삭제','rename':'파일 이름 변경'}
    journal.begin(op,{'kind':'file-edit','action':kind,'label':labels[kind]+f' {len(plans)}개','home':str(home),'plans':plans,'resources':['file:'+p['source'] for p in plans]})
    try:
        for index,plan in enumerate(plans):
            if cancelled():raise InterruptedError('파일 작업을 취소했습니다.')
            source=checked(plan['source']);target=checked(plan['target']);stamp=plan['signature'];stage=checked(plan['stage'])
            if signature(source)!=stamp or identity(source)!=plan['source_id']:raise ValueError('원본이 변경됐습니다: '+str(source))
            if target.exists():raise FileExistsError('대상에 새 항목이 생겼습니다: '+str(target))
            target.parent.mkdir(parents=True,exist_ok=True)
            if plan['mode']=='rename':
                journal.record(op,'publish-intent',{'index':index,'identity':identity(source)})
                source.rename(target)
            else:
                stage.parent.mkdir(parents=True,exist_ok=True)
                copy_item(source,stage,stamp,cancelled)
                journal.record(op,'publish-intent',{'index':index,'identity':identity(stage)})
                if target.exists():raise FileExistsError('대상에 새 항목이 생겼습니다: '+str(target))
                stage.rename(target)
                if plan['hold']:
                    if signature(source)!=stamp or identity(source)!=plan['source_id']:raise ValueError('이동 전에 원본이 바뀌었습니다. 원본은 유지됩니다.')
                    hold=checked(plan['hold']);hold.parent.mkdir(parents=True,exist_ok=True);source.rename(hold)
            journal.record(op,'editing',{'source':str(source)})
        journal.record(op,'completed',{})
        return OperationResult('completed',labels[kind]+' 완료','대화 연결 유지','Ctrl+Z 또는 작업 복구에서 되돌릴 수 있습니다.','해당 없음',report_id=op)
    except Exception as exc:
        journal.record(op,'needs_recovery',{'error':str(exc)})
        return OperationResult('needs_recovery','파일 작업 일부 완료','대화 연결 유지','작업 복구에서 확인하세요. 부분 복사본은 복구 보관소에 유지됩니다.','해당 없음',(str(exc),),report_id=op)


def undo_files(operation,journal):
    op=operation['id'];plans=operation['payload']['plans'];events=journal.events(op)
    if any(e['payload'].get('recovered') for e in events):return OperationResult('completed','이미 복구했습니다.','대화 유지','파일 유지',report_id=op)
    intents={e['payload']['index']:e['payload']['identity'] for e in events if e['phase']=='publish-intent'}
    owned=[]
    for index,plan in enumerate(plans):
        source=checked(plan['source']);target=checked(plan['target']);stamp=plan['signature'];stage=checked(plan['stage'])
        hold=checked(plan['hold']) if plan['hold'] else None
        target_owned=target.exists() and index in intents and identity(target)==intents[index]
        # An unpublished staging object or intact source means a competing target
        # was never acquired by this operation, even if its contents match.
        if plan['mode']=='rename' and source.exists():target_owned=False
        if plan['mode']!='rename' and stage.exists():target_owned=False
        if target_owned and signature(target)!=stamp:raise ValueError('대상 파일이 이후 변경됐습니다: '+str(target))
        if plan['mode']=='rename':
            if source.exists():
                if identity(source)!=plan['source_id'] or signature(source)!=stamp:raise ValueError('원래 위치에 새 항목이 있습니다: '+str(source))
            elif not target_owned:raise ValueError('복구할 원본 파일을 확인할 수 없습니다.')
        if hold:
            if hold.exists() and (source.exists() or identity(hold)!=plan['source_id'] or signature(hold)!=stamp):raise ValueError('복구 보관소 또는 원래 위치가 변경됐습니다.')
            if not hold.exists() and (not source.exists() or identity(source)!=plan['source_id'] or signature(source)!=stamp):raise ValueError('복구 원본을 확인할 수 없습니다.')
        owned.append(target_owned)
    for index in reversed(range(len(plans))):
        plan=plans[index];source=checked(plan['source']);target=checked(plan['target']);hold=checked(plan['hold']) if plan['hold'] else None
        if plan['mode']=='rename':
            if owned[index]:source.parent.mkdir(parents=True,exist_ok=True);target.rename(source)
        else:
            if hold and hold.exists():source.parent.mkdir(parents=True,exist_ok=True);hold.rename(source)
            if owned[index]:
                # Retain the verified copy instead of recursive removal, so an
                # interrupted undo never destroys part of the only recoverable tree.
                kept=checked(Path(plan['recovery'])/(str(index)+'-undone'))
                kept.parent.mkdir(parents=True,exist_ok=True)
                if kept.exists():raise ValueError('복구 보관소의 보관 경로가 이미 있습니다.')
                target.rename(kept)
        journal.record(op,'restoring-file',{'source':str(source)})
    journal.record(op,'completed',{'recovered':True})
    return OperationResult('completed','파일 작업 되돌리기 완료','대화 연결 유지','부분 복사본과 되돌린 복사본은 .codex-manager-file-recovery에 보관됩니다.','해당 없음',report_id=op)
