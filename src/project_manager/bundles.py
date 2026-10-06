import json
import os
import shutil
import uuid
from dataclasses import asdict
from datetime import datetime, timezone
from pathlib import Path
from .files import copy_verified, digest, safe_child, scan_roots, validate_paths, linked
from .models import Verification
from .codex.portability import transcript


def write_json(path, value):
    temporary=path.with_name(path.name+'.partial')
    with temporary.open('w',encoding='utf-8',newline='\n') as f:
        json.dump(value,f,ensure_ascii=False,indent=2,default=str);f.flush();os.fsync(f.fileno())
    os.replace(temporary,path)


def load_manifest(bundle):
    m=json.loads((bundle/'manifest.json').read_text(encoding='utf-8'))
    try:
        if not isinstance(m,dict) or m.get('format_version')!=1: raise ValueError('지원하지 않는 백업 형식입니다.')
        if m.get('state')!='complete': raise ValueError('완성되지 않은 백업입니다.')
        uuid.UUID(m['bundle_id'])
        project=m['project']
        if not isinstance(project,dict) or not isinstance(project['id'],str) or not isinstance(project['name'],str) or not isinstance(project['roots'],list): raise ValueError('프로젝트 정보가 올바르지 않습니다.')
        for key in ('roots','conversations','inventory'):
            if not isinstance(m[key],list) or not all(isinstance(x,dict) for x in m[key]): raise ValueError('백업 목록 형식이 올바르지 않습니다.')
        if not isinstance(m['dependencies'],list) or not isinstance(m['directories'],list): raise ValueError('백업 의존성·폴더 목록이 올바르지 않습니다.')
        for root in m['roots']:
            for key in ('id','original_path','bundle_path'):
                if not isinstance(root[key],str): raise ValueError('백업 폴더 경로가 올바르지 않습니다.')
        for t in m['conversations']:
            uuid.UUID(t['id'])
            for key in ('rollout','cwd','title'):
                if not isinstance(t[key],str): raise ValueError('대화 정보가 올바르지 않습니다.')
            if not isinstance(t['runtime_roots'],list): raise ValueError('대화의 작업 폴더 정보가 올바르지 않습니다.')
        for entry in m['inventory']:
            for key in ('root_id','relative_path','sha256','size'): entry[key]
        for directory in m['directories']:
            if not isinstance(directory,list) or len(directory)!=2: raise ValueError('폴더 목록 형식이 올바르지 않습니다.')
    except (KeyError,TypeError,AttributeError) as exc: raise ValueError('백업 매니페스트의 필수 정보가 잘못됐습니다.') from exc
    return m


def export_project(project, snapshot, destination, progress=lambda *_:None, cancelled=lambda:False,folder_memberships=None):
    if next((p for p in snapshot.projects if p.id==project.id),None)!=project:
        raise ValueError('프로젝트 연결이 바뀌었습니다. 새로고침 후 다시 백업하세요.')
    roots={f'root-{i+1:02d}':p for i,p in enumerate(project.roots)}
    errors=validate_paths(tuple(roots.values()),(destination,),())
    if errors: raise ValueError('\n'.join(errors))
    if destination.exists(): raise FileExistsError('백업 대상 폴더가 이미 있습니다. 새 폴더를 선택하세요.')
    folder_memberships=folder_memberships or {}
    folder_ids={t.id for t in snapshot.conversations if t.project_id is None and folder_memberships.get(t.id)==project.id}
    conversations=[t for t in snapshot.conversations if t.project_id==project.id or t.id in folder_ids]
    affected={t.id for t in conversations}
    while True:
        children=[t for t in snapshot.conversations if t.parent_id in affected and t.id not in affected]
        if not children: break
        affected.update(t.id for t in children);conversations.extend(children)
    inventory=scan_roots(roots,True)
    if inventory.blockers: raise ValueError('\n'.join(inventory.blockers))
    destination.mkdir(parents=True)
    manifest={'format_version':1,'bundle_id':str(uuid.uuid4()),'state':'partial',
              'captured_at':datetime.now(timezone.utc).isoformat(),'project':asdict(project),
              'roots':[{'id':rid,'original_path':str(p),'bundle_path':f'files/{rid}'} for rid,p in roots.items()],
              'conversations':[],'dependencies':[],'snapshot_revision':snapshot.revision,
              'inventory':[asdict(e) for e in inventory.entries], 'directories':inventory.directories,
              'folder_grouped_threads':sorted(folder_ids)}
    write_json(destination/'manifest.json',manifest)
    check=copy_verified(inventory,roots,{rid:destination/'files'/rid for rid in roots},progress,cancelled)
    if not check.ok: raise ValueError('\n'.join(check.errors))
    (destination/'conversations').mkdir()
    for t in conversations:
        if cancelled(): raise ValueError('백업을 취소했습니다.')
        if not t.rollout or not t.rollout.is_file(): raise ValueError(f'대화 기록이 없습니다: {t.title}')
        uuid.UUID(t.id)
        target=destination/'conversations'/f'{t.id}.jsonl'
        # A source chat can still append while the desktop is open. Detect changes.
        before=t.rollout.stat()
        shutil.copyfile(t.rollout,target)
        after=t.rollout.stat()
        if (before.st_size,before.st_mtime_ns)!=(after.st_size,after.st_mtime_ns) or digest(t.rollout)!=digest(target):
            raise ValueError(f'백업 중 대화가 바뀌었습니다: {t.title}')
        record=asdict(t);record['rollout']=target.relative_to(destination).as_posix()
        manifest['conversations'].append(record)
        if t.attachments:
            manifest['dependencies'].append({'kind':'thread_attachments','thread_id':t.id,'resolved':False,'metadata':t.attachments})
        if t.project_id!=project.id and t.id not in folder_ids: manifest['dependencies'].append({'kind':'cross_project_child','thread_id':t.id})
        if any(not any(path.resolve().is_relative_to(root.resolve()) for root in project.roots) for path in (t.cwd,*t.runtime_roots)):
            manifest['dependencies'].append({'kind':'external_workspace','thread_id':t.id,'resolved':False})
        # Only explicit image/attachment file references; command text is history.
        for line in target.open(encoding='utf-8'):
            item=json.loads(line)
            if item.get('type')!='response_item': continue
            for part in item.get('payload',{}).get('content',[]) or []:
                if not isinstance(part,dict): continue
                raw=part.get('path') or part.get('image_path')
                if isinstance(raw,str) and Path(raw).is_absolute():
                    outside=not any(Path(raw).resolve().is_relative_to(p.resolve()) for p in roots.values())
                    if outside: manifest['dependencies'].append({'kind':'external_attachment','path':raw,'resolved':False})
    for root in project.roots:
        git=root/'.git'
        if git.is_file(): manifest['dependencies'].append({'kind':'linked_worktree','path':str(root),'resolved':False})
    manifest['state']='complete'
    write_json(destination/'manifest.json',manifest)
    checksum=[]
    for folder,_,names in os.walk(destination):
        for name in names:
            p=Path(folder)/name
            if p.parent==destination and p.name in ('checksums.jsonl','verification.json'): continue
            checksum.append({'path':p.relative_to(destination).as_posix(),'size':p.stat().st_size,'sha256':digest(p)})
    with (destination/'checksums.jsonl').open('x',encoding='utf-8') as f:
        for entry in checksum: f.write(json.dumps(entry,ensure_ascii=False)+'\n')
        f.flush();os.fsync(f.fileno())
    verification=verify_bundle(destination)
    write_json(destination/'verification.json',{'integrity':verification.ok,'restore_verified':False,'errors':verification.errors})
    if not verification.ok: raise ValueError('\n'.join(verification.errors))
    return verification


def verify_bundle(bundle: Path):
    try:
        if linked(bundle): raise ValueError('링크된 백업 폴더는 사용할 수 없습니다.')
        m=load_manifest(bundle)
        expected={}
        for line in (bundle/'checksums.jsonl').open(encoding='utf-8'):
            entry=json.loads(line);path=safe_child(bundle,entry['path'])
            if entry['path'] in expected: raise ValueError('백업 파일 목록에 중복 경로가 있습니다.')
            expected[entry['path']]=entry
            if not path.is_file() or path.stat().st_size!=entry['size'] or digest(path)!=entry['sha256']:
                raise ValueError(f"백업 파일 검증 실패: {entry['path']}")
        if 'manifest.json' not in expected: raise ValueError('매니페스트 해시가 없습니다.')
        actual=set()
        for folder,dirs,names in os.walk(bundle):
            for name in (*dirs,*names): safe_child(bundle,(Path(folder)/name).relative_to(bundle).as_posix())
            for name in names:
                relative=(Path(folder)/name).relative_to(bundle).as_posix()
                if relative not in ('checksums.jsonl','verification.json'): actual.add(relative)
        if actual!=set(expected): raise ValueError('백업에 검증 목록과 다른 파일이 있습니다.')
        declared={'manifest.json'}
        root_ids={r['id'] for r in m['roots']}
        if len(root_ids)!=len(m['roots']): raise ValueError('중복 루트 ID입니다.')
        for r in m['roots']:
            safe_child(bundle,r['bundle_path'])
            if r['bundle_path']!=f"files/{r['id']}": raise ValueError('루트 폴더 경로가 올바르지 않습니다.')
        for t in m['conversations']:
            safe_child(bundle,t['rollout'])
            if t['rollout'] not in expected: raise ValueError('대화 기록의 해시가 없습니다.')
            declared.add(t['rollout'])
        for e in m['inventory']:
            name=f"files/{e['root_id']}/{e['relative_path']}"
            safe_child(bundle,name)
            if name not in expected: raise ValueError('파일의 해시가 없습니다.')
            if e['root_id'] not in root_ids or expected[name]['sha256']!=e['sha256'] or expected[name]['size']!=e['size']:
                raise ValueError('매니페스트와 검증 목록이 일치하지 않습니다.')
            declared.add(name)
        if declared!=set(expected): raise ValueError('매니페스트에 없는 파일이 검증 목록에 있습니다.')
        return Verification(True,evidence_path=bundle/'verification.json')
    except (OSError,ValueError,KeyError,TypeError) as exc: return Verification(False,(str(exc),))


def read_transcript(bundle, thread_id):
    m=load_manifest(bundle)
    t=next(t for t in m['conversations'] if t['id']==thread_id)
    return list(transcript(safe_child(bundle,t['rollout'])))
