"""Disposable isolated protocol probe. Never connects to the live app."""
import json
import sys
import uuid
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))
from project_manager.codex.rpc import isolated_client


def main():
    base = Path('.test-artifacts') / ('probe-' + uuid.uuid4().hex)
    root = (base / 'files').resolve(); root.mkdir(parents=True)
    with isolated_client(base / 'home-a') as client:
        for method, params in [('project/create', {'idempotencyKey':uuid.uuid4().hex,'name':'probe','roots':[{'path':str(root)}]}),
                               ('project/list', {}), ('thread/list', {})]:
            try:
                print(method, json.dumps(client.call(method,params),ensure_ascii=False))
            except Exception as exc:
                print(type(exc).__name__,str(exc))


if __name__ == '__main__': main()
