import json
import os
import queue
import subprocess
import threading
from pathlib import Path
from ..version import VERSION


class RpcError(RuntimeError):
    pass


class CodexClient:
    def __init__(self, command, env=None, initialize=True):
        self.command, self.env, self.initialize = command, env, initialize
        self.requests_sent = 0
        self.lock = threading.RLock()
        self.responses = queue.Queue()
        self.notifications = queue.Queue()
        self.process = None

    def __enter__(self):
        self.process = subprocess.Popen(
            self.command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, text=True, encoding='utf-8',
            env=self.env, creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
        threading.Thread(target=self._read, daemon=True).start()
        try:
            if self.initialize:
                self.call('initialize', {'clientInfo': {'name': 'codex_manager', 'version': VERSION},
                                        'capabilities': {'experimentalApi': True}})
                self.process.stdin.write(json.dumps({'method': 'initialized'}) + '\n')
                self.process.stdin.flush()
        except BaseException:
            self.__exit__();raise
        return self

    def _read(self):
        try:
            for line in self.process.stdout:
                try:
                    item = json.loads(line)
                except ValueError:
                    continue
                (self.responses if 'id' in item else self.notifications).put(item)
        finally:
            self.responses.put({'disconnected': True})

    def call(self, method, params, timeout=30):
        with self.lock:
            self.requests_sent += 1
            request_id = self.requests_sent
            self.process.stdin.write(json.dumps({'id': request_id, 'method': method, 'params': params}) + '\n')
            self.process.stdin.flush()
            import time
            deadline = time.monotonic() + timeout
            while True:
                try:
                    response = self.responses.get(timeout=max(0, deadline-time.monotonic()))
                except queue.Empty:
                    raise TimeoutError(f'{method}: 응답 시간 초과. 변경 결과를 다시 확인해야 합니다.') from None
                if response.get('disconnected'):
                    raise RpcError('Codex 연결이 종료되었습니다.')
                if response.get('id') != request_id:
                    continue  # late result from an earlier timed-out request
                if 'error' in response:
                    raise RpcError(f"{method}: {response['error'].get('message', response['error'])}")
                return response.get('result', {})

    def __exit__(self, *args):
        if self.process:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
            for stream in (self.process.stdin, self.process.stdout):
                if stream:
                    stream.close()


def find_cli():
    import shutil
    bundled=Path(os.environ.get('LOCALAPPDATA',''))/'OpenAI/Codex/bin'
    candidates=list(bundled.glob('*/codex.exe'))
    if candidates:
        return str(max(candidates,key=lambda p:p.stat().st_mtime))
    installed = Path(os.environ.get('LOCALAPPDATA', '')) / 'Programs/OpenAI/Codex/bin/codex.exe'
    if installed.exists():
        return str(installed)
    found = shutil.which('codex')
    if found:
        return found
    raise FileNotFoundError('Codex 실행 파일을 찾을 수 없습니다.')


def isolated_client(home: Path):
    home.mkdir(parents=True, exist_ok=True)
    (home / 'sessions').mkdir(exist_ok=True)
    (home / 'archived_sessions').mkdir(exist_ok=True)
    env = os.environ.copy()
    env['CODEX_HOME'] = str(home.resolve())
    env['CODEX_SQLITE_HOME'] = str((home / 'sqlite').resolve())
    env['RUST_LOG'] = 'error'
    return CodexClient([find_cli(),'-c','sqlite_home='+json.dumps(str((home/'sqlite').resolve())), 'app-server', '--listen', 'stdio://'], env)
