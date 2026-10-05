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
        snapshot=adapter.snapshot();threads={t.id:t for t in snapshot.conversations}
        completed=[]
        for entry in m['inventory']:
            path=safe_child(Path(report['destinations'][entry['root_id']]),entry['relative_path'])
            if not path.is_file() or digest(path)!=entry['sha256']: raise ValueError(f'복원 파일이 다르거나 없습니다: {path}')
        for original in m['conversations']:
            restored=threads.get(original['id'])
            if not restored or restored.project_id!=report['project_id'] or not restored.rollout:
                raise ValueError('복원된 대화 연결을 확인할 수 없습니다.')
            before=read_transcript(bundle,original['id']);after=list(transcript(restored.rollout))
            if after[:len(before)]!=before: raise ValueError('복원된 대화 내용이 백업과 다릅니다.')
            with restored.rollout.open('rb') as f:
                f.seek(report['threads'][original['id']]['offset'])
                new_records=[json.loads(line) for line in f if line.strip()]
            if any(x.get('type')=='event_msg' and x.get('payload',{}).get('type')=='task_complete' for x in new_records):
                completed.append(original['id'])
        if not completed: raise ValueError('복원한 대화를 Codex에서 한 번 이어 사용한 뒤 다시 확인하세요.')
        proof={'integrity':True,'restore_verified':True,'bundle_id':m['bundle_id'],
               'tested_thread_ids':completed,'verified_at':datetime.now(timezone.utc).isoformat(),
               'target_project_id':report['project_id'],'target_home':str(adapter.home)}
        write_json(bundle/'verification.json',proof)
        return Verification(True,evidence_path=bundle/'verification.json')
    except (OSError,ValueError,KeyError,StopIteration) as exc: return Verification(False,(str(exc),))
