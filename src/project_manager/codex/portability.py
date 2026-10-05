import json
import os
import uuid
from datetime import datetime, timezone, timedelta
from pathlib import Path


def prepare_rollout(source: Path, home: Path, cwd: Path, runtime_roots: tuple[Path,...]) -> Path:
    """Make a derived import file. Preserve historical messages and turn contexts."""
    with source.open(encoding='utf-8') as stream:
        first = json.loads(stream.readline())
    if first.get('type') != 'session_meta':
        raise ValueError('대화 첫 기록이 session_meta가 아닙니다.')
    tid = str(uuid.UUID(first['payload']['id']))
    timestamp = datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%S')
    day=timestamp[:10]
    target_dir = home / 'sessions' / day[:4] / day[5:7] / day[8:10]
    target_dir.mkdir(parents=True, exist_ok=True)
    target = target_dir / f"rollout-{timestamp.replace(':','-')}-{tid}.jsonl"
    while target.exists():
        timestamp=(datetime.fromisoformat(timestamp)+timedelta(seconds=1)).strftime('%Y-%m-%dT%H:%M:%S')
        target=target_dir/f"rollout-{timestamp.replace(':','-')}-{tid}.jsonl"
    temporary = target.with_suffix('.partial')
    first['payload']['cwd'] = str(cwd.resolve())
    first['payload']['runtime_workspace_roots'] = [str(p.resolve()) for p in runtime_roots]
    try:
        with source.open(encoding='utf-8') as src, temporary.open('x', encoding='utf-8', newline='\n') as dst:
            src.readline()
            dst.write(json.dumps(first, ensure_ascii=False) + '\n')
            for line in src:
                dst.write(line)
            dst.flush(); os.fsync(dst.fileno())
        os.replace(temporary, target)
    except Exception:
        temporary.unlink(missing_ok=True)
        raise
    return target


def transcript(source: Path):
    for line in source.open(encoding='utf-8'):
        item = json.loads(line)
        payload = item.get('payload',{})
        if item.get('type') == 'response_item' and payload.get('type') == 'message':
            role = payload.get('role','')
            if role not in ('user','assistant'):
                continue
            text = '\n'.join(part.get('text','') for part in payload.get('content',[]) if isinstance(part,dict))
            if text:
                yield {'role':role,'text':text}
