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


def run_projects(adapter,plans,target,clean,journal,inferred,progress,cancelled):
    completed=[];errors=[]
    for project,mapping in plans:
        if cancelled():errors.append('취소했습니다. 시작하지 않은 프로젝트는 그대로 유지됩니다.');break
        try:
            snapshot=adapter.snapshot()
            if next((p for p in snapshot.projects if p.id==project.id),None)!=project:
                raise ValueError('프로젝트가 변경됐습니다. 새로고침 후 다시 진행하세요: '+project.name)
            current_target=None
            if target:
                current_target=next((p for p in snapshot.projects if p.id==target.id),None)
                if current_target!=target:raise ValueError('대상 프로젝트가 변경됐습니다.')
            recovery=next(iter(mapping.values())).parent/'.project-manager-recovery'
            result=transfer_with_connections(adapter,project,mapping,journal,recovery,current_target,clean,inferred.get(project.id,()),progress,cancelled)
            if result.state!='completed':
                errors.extend(result.errors or (result.file_status,));break
            completed.append(project.name)
        except Exception as exc:errors.append(str(exc));break
    return OperationResult('needs_recovery' if errors else 'completed',f'프로젝트 {len(completed)}/{len(plans)}개 완료',
        '완료한 프로젝트의 연결 변경 유지','완료한 작업은 유지합니다. 중단된 작업은 작업 복구에서 확인하세요.' if errors else ('원본 정리 완료' if clean else '원본 파일 유지'),
        '모바일 미확인',tuple(errors))
