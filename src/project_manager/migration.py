"""Discover portable project folders and plan non-overwriting imports."""
import json
import os
import re
from dataclasses import dataclass,replace
from pathlib import Path
from .bundles import load_manifest
from .files import linked, validate_paths
from .models import OperationResult,Project


@dataclass(frozen=True)
class BundleInfo:
    path: Path
    manifest: dict


@dataclass(frozen=True)
class Discovery:
    bundles: tuple[BundleInfo, ...]
    errors: tuple[str, ...]


@dataclass(frozen=True)
class ImportPlan:
    bundle: BundleInfo
    destinations: dict[str, Path]
    target_id: str | None
    duplicates: tuple[str, ...]
    target: Project | None = None


def discover_bundles(folder, cancelled=lambda: False, limit=20000):
    folder = Path(folder).absolute()
    if not folder.is_dir(): raise ValueError('백업 폴더를 찾을 수 없습니다: '+str(folder))
    if any(linked(p) for p in (folder, *folder.parents)): raise ValueError('링크된 폴더는 검색할 수 없습니다.')
    found=[]; errors=[]; count=0
    for current, dirs, files in os.walk(folder, followlinks=False, onerror=lambda e: errors.append(str(e))):
        if cancelled(): raise ValueError('백업 검색을 취소했습니다.')
        count+=1
        if count>limit:
            errors.append('검색 범위가 큽니다. 아직 찾지 못한 백업은 더 안쪽 폴더를 선택해 검색하세요.'); break
        root=Path(current); dirs[:]=sorted(d for d in dirs if not linked(root/d))
        if 'manifest.json' not in files: continue
        try:
            raw=json.loads((root/'manifest.json').read_text(encoding='utf-8'))
            if not isinstance(raw,dict) or 'bundle_id' not in raw: continue
            dirs[:]=[]  # A bundle's own files are never separate imports.
            found.append(BundleInfo(root,load_manifest(root)))
        except (OSError,ValueError,TypeError) as exc:
            errors.append(str(root)+': '+str(exc))
    return Discovery(tuple(found),tuple(errors))


def folder_name(value):
    value=re.sub(r'[<>:"/\\|?*\x00-\x1f]','_',value).strip('. ')[:90] or 'project'
    if re.fullmatch(r'(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\..*)?',value,re.I): value='_'+value
    return value


def plan_import(bundles,snapshot,parent,target_id=None):
    parent=Path(parent).resolve(); reserved=set(); result=[]; existing={t.id for t in snapshot.conversations}
    if target_id and not any(p.id==target_id for p in snapshot.projects): raise ValueError('합칠 프로젝트가 없습니다.')
    for bundle in bundles:
        m=bundle.manifest; name=folder_name(m['project']['name']); base=parent/name; n=2
        while base.exists() or str(base).casefold() in reserved:
            base=parent/(name+f' ({n})'); n+=1
        reserved.add(str(base).casefold())
        dest={r['id']:base if len(m['roots'])==1 else base/(f'{i+1:02d}-'+folder_name(Path(r['original_path'].replace('\\','/')).name)) for i,r in enumerate(m['roots'])}
        errors=validate_paths((bundle.path,),tuple(dest.values()),())
        if errors: raise ValueError('\n'.join(errors))
        ids={t['id'] for t in m['conversations']}
        target=next((p for p in snapshot.projects if p.id==target_id),None)
        result.append(ImportPlan(bundle,dest,target_id,tuple(sorted(ids & existing)),target))
        existing.update(ids)  # Also surface overlaps between selected backups.
    return tuple(result)


def run_imports(plans,adapter,journal,skip_existing,progress=lambda *_:None,cancelled=lambda:False):
    from .operations import import_bundle
    from .progress import item as progress_item
    items=[{'project_id':p.bundle.manifest['project']['id'],'name':p.bundle.manifest['project']['name'],
            'sources':[str(p.bundle.path)],'destinations':list(map(str,p.destinations.values())),
            'status':'not_started','errors':[]} for p in plans]
    report={'home':str(adapter.home),'action':'import','label':'다른 컴퓨터의 작업 가져오기','result_state':'running','items':items}
    report_id=journal.report(report); errors=[]; stopped=False
    expected_targets={p.target_id:p.target for p in plans if p.target_id}
    for index,plan in enumerate(plans):
        if cancelled(): stopped=True; break
        progress_item(progress,index+1,len(plans),plan.bundle.manifest['project']['name'])
        item=items[index];item['status']='in_progress';journal.update_report(report_id,report)
        try:
            if any(path.exists() for path in plan.destinations.values()):raise ValueError('예정한 저장 폴더가 이미 있습니다. 가져오기 화면에서 새 위치를 확인하세요.')
            snapshot=adapter.snapshot();target=None
            if plan.target_id:
                target=next((p for p in snapshot.projects if p.id==plan.target_id),None)
                expected=expected_targets[plan.target_id]
                if target is None or expected is None or replace(target,legacy_ids=expected.legacy_ids)!=expected: raise ValueError('합칠 프로젝트가 미리보기 이후 변경됐습니다. 다시 선택하세요.')
            for other in snapshot.projects:
                if other.id!=plan.target_id and any(d.is_relative_to(r.resolve()) or r.resolve().is_relative_to(d) for d in plan.destinations.values() for r in other.roots):
                    raise ValueError('다른 프로젝트가 사용하는 저장 위치입니다: '+other.name)
            result=import_bundle(plan.bundle.path,plan.destinations,adapter,journal.for_item(report_id,index),progress,cancelled,
                                 target=target,skip_existing=skip_existing)
            item['status']=result.state;item['errors']=list(result.errors)
            if result.state!='completed': errors.extend(result.errors); break
            if target:
                extra=tuple(d for d in plan.destinations.values() if not any(d.is_relative_to(r.resolve()) for r in target.roots))
                expected_targets[target.id]=replace(target,roots=target.roots+extra)
            journal.update_report(report_id,report)
        except Exception as exc:
            item['status']='failed';item['errors']=[str(exc)];errors.append(str(exc));break
    state='needs_recovery' if any(i['status']=='needs_recovery' for i in items) else 'failed' if errors else 'cancelled' if stopped else 'completed'
    report['result_state']=state;journal.update_report(report_id,report)
    count=sum(i['status']=='completed' for i in items)
    return OperationResult(state,f'프로젝트 {count}/{len(plans)}개 가져오기 완료','대화와 파일 등록 결과를 확인하세요.',
                           '외장하드의 백업은 그대로 보관됩니다.',errors=tuple(errors),report_id=report_id)
