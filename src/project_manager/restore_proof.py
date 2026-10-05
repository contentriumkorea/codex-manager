import json
from datetime import datetime,timezone
from pathlib import Path
from .bundles import verify_bundle,load_manifest,read_transcript,write_json
from .codex.portability import transcript
from .files import safe_child,digest
from .models import Verification


def verify_restored_bundle(bundle,adapter):
    check=verify_bundle(bundle)
    if not check.ok: return check
    try:
        m=load_manifest(bundle)
        report_path=adapter.home/'project-manager-import-reports'/(m['bundle_id']+'.json')
        if not report_path.exists(): raise ValueError('이 컴퓨터에 해당 백업을 가져온 기록이 없습니다.')
        report=json.loads(report_path.read_text(encoding='utf-8'))
        if report.get('manifest_digest')!=digest(bundle/'manifest.json'): raise ValueError('다른 백업의 복원 기록입니다.')
        snapshot=adapter.snapshot();threads={t.id:t for t in snapshot.conversations}
        project=next((p for p in snapshot.projects if p.id==report['project_id']),None)
        if not project or tuple(p.resolve() for p in project.roots)!=tuple(Path(report['destinations'][r['id']]).resolve() for r in m['roots']):
            raise ValueError('복원된 프로젝트의 폴더 연결이 바뀌었습니다.')
        from .operations import map_path
        from .catalog import clean_path
        mapping={clean_path(r['original_path']):Path(report['destinations'][r['id']]) for r in m['roots']}
        completed=[]
        for entry in m['inventory']:
            path=safe_child(Path(report['destinations'][entry['root_id']]),entry['relative_path'])
            if not path.is_file() or digest(path)!=entry['sha256']: raise ValueError(f'복원 파일이 다르거나 없습니다: {path}')
        for original in m['conversations']:
            restored=threads.get(original['id'])
            if not restored or restored.project_id!=report['project_id'] or not restored.rollout:
                raise ValueError('복원된 대화 연결을 확인할 수 없습니다.')
            cwd=map_path(clean_path(original['cwd']),mapping)
            runtime=tuple(map_path(clean_path(p),mapping) for p in original['runtime_roots']) or (cwd,)
            if (restored.cwd.resolve()!=cwd.resolve() or tuple(p.resolve() for p in restored.runtime_roots)!=tuple(p.resolve() for p in runtime) or
                restored.parent_id!=original['parent_id']): raise ValueError('복원된 대화의 작업 폴더·부모 연결이 다릅니다.')
            before=read_transcript(bundle,original['id']);after=list(transcript(restored.rollout))
            if after[:len(before)]!=before: raise ValueError('복원된 대화 내용이 백업과 다릅니다.')
            with restored.rollout.open('rb') as f:
                f.seek(report['threads'][original['id']]['offset'])
                new_records=[json.loads(line) for line in f if line.strip()]
            started=None;user=False;assistant=False;complete=False
            for record in new_records:
                payload=record.get('payload',{})
                kind=payload.get('type') if record.get('type')=='event_msg' else None
                if kind=='task_started': started=payload.get('turn_id');user=False;assistant=False
                elif started and kind=='user_message': user=True
                elif started and record.get('type')=='response_item' and payload.get('type')=='message':
                    if payload.get('role')=='user': user=True
                    if payload.get('role')=='assistant' and payload.get('content'): assistant=True
                elif kind=='task_complete' and started and payload.get('turn_id')==started and user and assistant: complete=True
            if complete:
                completed.append(original['id'])
        if not completed: raise ValueError('복원한 대화를 Codex에서 한 번 이어 사용한 뒤 다시 확인하세요.')
        proof={'integrity':True,'restore_verified':True,'bundle_id':m['bundle_id'],'manifest_digest':digest(bundle/'manifest.json'),
               'tested_thread_ids':completed,'verified_at':datetime.now(timezone.utc).isoformat(),
               'target_project_id':report['project_id'],'target_home':str(adapter.home)}
        write_json(bundle/'verification.json',proof)
        return Verification(True,evidence_path=bundle/'verification.json')
    except (OSError,ValueError,KeyError,StopIteration) as exc: return Verification(False,(str(exc),))
