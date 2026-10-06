import sys
import pytest
from project_manager.codex.rpc import CodexClient, RpcError


def test_response_and_notifications_are_separated(tmp_path):
    server = tmp_path / 'server.py'
    server.write_text('''import sys,json
for line in sys.stdin:
 x=json.loads(line)
 if 'id' in x:
  print(json.dumps({'method':'changed','params':{}}),flush=True)
  print(json.dumps({'id':x['id'],'result':{'ok':True}}),flush=True)
''')
    with CodexClient([sys.executable,str(server)]) as client:
        assert client.call('read', {}) == {'ok': True}


def test_timeout_after_mutation_requires_readback(tmp_path):
    server = tmp_path / 'server.py'
    server.write_text('import sys,time\nfor line in sys.stdin: time.sleep(5)')
    with CodexClient([sys.executable,str(server)], initialize=False) as client:
        with pytest.raises(TimeoutError):
            client.call('project/create', {}, timeout=.05)
        assert client.requests_sent == 1


def test_initialization_failure_reaps_server_and_closes_pipes(tmp_path):
    server=tmp_path/'server.py'
    server.write_text("import sys,json\nfor line in sys.stdin:\n x=json.loads(line)\n if 'id' in x: print(json.dumps({'id':x['id'],'error':{'message':'initialization failed'}}),flush=True)\n")
    client=CodexClient([sys.executable,str(server)])
    try:
        with pytest.raises(RpcError): client.__enter__()
        assert client.process.poll() is not None
        assert client.process.stdin.closed and client.process.stdout.closed
    finally: client.__exit__()
