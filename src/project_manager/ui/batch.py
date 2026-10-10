"""Sequential, individually journaled project transfers with one confirmation."""
import re
from pathlib import Path
from ..models import OperationResult
from .flows import transfer_with_connections


def plan_projects(projects,parent):
    parent=Path(parent);reserved=set();plans=[]
    for project in projects:
        if not project.roots:raise ValueError('등록된 폴더가 없습니다: '+project.name)
        name=re.sub(r'[<>:"/\\|?*]','_',project.name).strip('. ') or 'project'
        base=parent/name;n=2
        while base.exists() or str(base).casefold() in reserved:
            base=parent/(name+f' ({n})');n+=1
        reserved.add(str(base).casefold());mapping={}
        for index,root in enumerate(project.roots):
            target=base if len(project.roots)==1 else base/(str(index+1)+'-'+root.name)
            if any(target.resolve().is_relative_to(r.resolve()) for p in projects for r in p.roots):
                raise ValueError('원본 프로젝트 폴더 안으로 이동할 수 없습니다.')
            mapping[str(root)]=target
        plans.append((project,mapping))
    return plans


def run_projects(adapter,plans,target,clean,journal,inferred,progress,cancelled,retry_of=None):
    completed=[];errors=[];stopped=False
    items=[{'project_id':p.id,'name':p.name,'sources':list(map(str,p.roots)),'destinations':list(map(str,mapping.values())),'status':'not_started','errors':[]} for p,mapping in plans]
    report={'home':str(adapter.home),'action':'merge' if target else 'move','label':'프로젝트 합치기' if target else '프로젝트 옮기기','result_state':'running','items':items,'target_id':target.id if target else None,'target_name':target.name if target else None,'clean':clean}
    report['retry_of']=retry_of;report_id=journal.report(report)
    for index,(project,mapping) in enumerate(plans):
        if cancelled():stopped=True;break
        item=items[index]
        item['status']='in_progress';journal.update_report(report_id,report)
        try:
            snapshot=adapter.snapshot()
            if next((p for p in snapshot.projects if p.id==project.id),None)!=project:
                raise ValueError('프로젝트가 변경됐습니다. 새로고침 후 다시 진행하세요: '+project.name)
            current_target=None
            if target:
                current_target=next((p for p in snapshot.projects if p.id==target.id),None)
                if current_target!=target:raise ValueError('대상 프로젝트가 변경됐습니다.')
            recovery=next(iter(mapping.values())).parent/'.project-manager-recovery'
            result=transfer_with_connections(adapter,project,mapping,journal.for_item(report_id,index),recovery,current_target,clean,inferred.get(project.id,()),progress,cancelled)
            if result.state!='completed':
                item['status']='needs_recovery';item['errors']=list(result.errors or (result.file_status,));errors.extend(item['errors']);break
            item['status']='completed'
            completed.append(project.name)
            journal.update_report(report_id,report)
        except Exception as exc:
            item['status']='failed';item['errors']=[str(exc)];errors.append(str(exc));break
    state='needs_recovery' if errors else 'cancelled' if stopped else 'completed'
    report['result_state']=state;journal.update_report(report_id,report)
    return OperationResult(state,f'프로젝트 {len(completed)}/{len(plans)}개 완료',
        '완료한 프로젝트의 연결 변경 유지','완료한 작업은 유지합니다. 미완료 항목은 작업 내역에서 확인하세요.' if state!='completed' else ('원본 정리 완료' if clean else '원본 파일 유지'),
        '모바일 미확인',tuple(errors),report_id)


def run_backups(adapter,plans,journal,export,progress,cancelled,retry_of=None):
    from ..grouping import display_membership
    from ..settings import read_settings
    items=[{'project_id':p.id,'name':p.name,'sources':list(map(str,p.roots)),'destinations':[str(destination)],'status':'not_started','errors':[]} for p,destination in plans]
    errors=[];stopped=False
    report={'home':str(adapter.home),'action':'backup','label':'프로젝트 백업','result_state':'running','items':items}
    report['retry_of']=retry_of;report_id=journal.report(report)
    for item,(project,destination) in zip(items,plans):
        if cancelled():stopped=True;break
        item['status']='in_progress';journal.update_report(report_id,report)
        try:
            snapshot=adapter.snapshot()
            membership=display_membership(snapshot,read_settings(adapter.home/'.codex-global-state.json'))
            check=export(project,snapshot,destination,progress,cancelled,folder_memberships=membership.inferred)
            if not check.ok:raise ValueError('\n'.join(check.errors))
            item['status']='completed'
            journal.update_report(report_id,report)
        except Exception as exc:
            item['status']='failed';item['errors']=[str(exc)];errors.append(str(exc));break
    state='failed' if errors else 'cancelled' if stopped else 'completed'
    report['result_state']=state;journal.update_report(report_id,report)
    count=sum(item['status']=='completed' for item in items)
    return OperationResult(state,f'프로젝트 {count}/{len(plans)}개 백업 완료','대화와 파일을 함께 보관','원본 파일과 대화 유지','해당 없음',tuple(errors),report_id)
