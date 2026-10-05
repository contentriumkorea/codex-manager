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
    if m.get('format_version')!=1: raise ValueError('지원하지 않는 백업 형식입니다.')
    if m.get('state')!='complete': raise ValueError('완성되지 않은 백업입니다.')
    uuid.UUID(m['bundle_id'])
    return m


def export_project(project, snapshot, destination, progress=lambda *_:None, cancelled=lambda:False):
    roots={f'root-{i+1:02d}':p for i,p in enumerate(project.roots)}
    errors=validate_paths(tuple(roots.values()),(destination,),())
    if errors: raise ValueError('\n'.join(errors))
    if destination.exists(): raise FileExistsError('백업 대상 폴더가 이미 있습니다. 새 폴더를 선택하세요.')
    conversations=[t for t in snapshot.conversations if t.project_id==project.id]
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
              'inventory':[asdict(e) for e in inventory.entries], 'directories':inventory.directories}
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
        if t.project_id!=project.id: manifest['dependencies'].append({'kind':'cross_project_child','thread_id':t.id})
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
            if p.name in ('checksums.jsonl','verification.json'): continue
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
        for r in m['roots']:
            safe_child(bundle,r['bundle_path'])
        for t in m['conversations']:
            safe_child(bundle,t['rollout'])
            if t['rollout'] not in expected: raise ValueError('대화 기록의 해시가 없습니다.')
        for e in m['inventory']:
            name=f"files/{e['root_id']}/{e['relative_path']}"
            safe_child(bundle,name)
            if name not in expected: raise ValueError('파일의 해시가 없습니다.')
        return Verification(True,evidence_path=bundle/'verification.json')
    except (OSError,ValueError,KeyError,TypeError) as exc: return Verification(False,(str(exc),))


def read_transcript(bundle, thread_id):
    m=load_manifest(bundle)
    t=next(t for t in m['conversations'] if t['id']==thread_id)
    return list(transcript(safe_child(bundle,t['rollout'])))
