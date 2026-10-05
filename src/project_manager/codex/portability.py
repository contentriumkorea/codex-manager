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
    last_context=-1;last_environment=-1;last_settings=-1
    with source.open(encoding='utf-8') as stream:
        for index,line in enumerate(stream):
            item=json.loads(line)
            if item.get('type')=='turn_context': last_context=index
            if item.get('type')=='event_msg' and item.get('payload',{}).get('type')=='thread_settings_applied': last_settings=index
            if item.get('type')=='world_state' and 'cwd' in item.get('payload',{}).get('state',{}).get('environments',{}).get('environments',{}).get('local',{}):
                last_environment=index
    first['payload']['cwd'] = str(cwd.resolve())
    first['payload']['runtime_workspace_roots'] = [str(p.resolve()) for p in runtime_roots]
    try:
        with source.open(encoding='utf-8') as src, temporary.open('x', encoding='utf-8', newline='\n') as dst:
            src.readline()
            dst.write(json.dumps(first, ensure_ascii=False) + '\n')
            for index,line in enumerate(src,start=1):
                if index==last_context:
                    context=json.loads(line)
                    context['payload']['cwd']=str(cwd.resolve())
                    context['payload']['runtime_workspace_roots']=[str(p.resolve()) for p in runtime_roots]
                    dst.write(json.dumps(context,ensure_ascii=False)+'\n')
                elif index==last_environment:
                    state=json.loads(line)
                    local=state['payload']['state']['environments']['environments']['local']
                    local['cwd']=str(cwd.resolve())
                    dst.write(json.dumps(state,ensure_ascii=False)+'\n')
                elif index==last_settings:
                    event=json.loads(line);settings=event['payload']['thread_settings']
                    settings['cwd']=str(cwd.resolve());settings['runtime_workspace_roots']=[str(p.resolve()) for p in runtime_roots]
                    dst.write(json.dumps(event,ensure_ascii=False)+'\n')
                else: dst.write(line)
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
            kinds=payload.get('internal_chat_message_metadata_passthrough',{}).get('content_item_kinds',[])
            text = '\n'.join(part.get('text','') for i,part in enumerate(payload.get('content',[]))
                             if isinstance(part,dict) and (role!='user' or not kinds or i>=len(kinds) or kinds[i]=='user.text'))
            if text:
                yield {'role':role,'text':text}
