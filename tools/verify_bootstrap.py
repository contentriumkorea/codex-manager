"""Native, isolated acceptance tests for the unchanged online installer."""
import argparse
import configparser
import ctypes
from functools import partial
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time
import zipfile

from build_bootstrap import build_bootstrap
from verify_setup import registration, verify as verify_real_setup

ASSET = 'Codex-Manager-Windows-x64.zip'


class Channel:
    def __init__(self):
        self.files = {}
        self.requests = []

    def release(self, version):
        files = {'CodexManager.exe': (Path(os.environ['WINDIR']) / 'System32/where.exe').read_bytes(),
                 '_internal/fixture.txt': version.encode(), '_internal/project_manager/ui/app-icon.ico':
                 (Path(__file__).resolve().parents[1] / 'src/project_manager/ui/app-icon.ico').read_bytes()}
        files['install-manifest.json'] = json.dumps({'version': version, 'files':
            {k: hashlib.sha256(v).hexdigest() for k, v in files.items()}}).encode()
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, 'w', zipfile.ZIP_DEFLATED) as z:
            for name, data in files.items():
                z.writestr('CodexManager/' + name, data)
        payload = buffer.getvalue()
        self.files = {'/releases/latest/download/install.ini':
            (f'[release]\nprotocol=1\nversion={version}\nsize={len(payload)}\n'
             f'sha256={hashlib.sha256(payload).hexdigest()}\nexpanded_size={sum(map(len, files.values()))}\n').encode(),
            f'/releases/download/v{version}/{ASSET}': payload}


class Handler(BaseHTTPRequestHandler):
    def __init__(self, *args, channel, **kwargs):
        self.channel = channel
        super().__init__(*args, **kwargs)

    def do_GET(self):
        self.channel.requests.append(self.path)
        body = self.channel.files.get(self.path)
        if body is None:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        # Pace fixture writes: this host's loopback filter truncates coalesced
        # writes above 64 KiB (also reproduced with curl and urllib clients).
        for offset in range(0, len(body), 16384):
            self.wfile.write(body[offset:offset + 16384])
            self.wfile.flush()
            time.sleep(.002)

    def log_message(self, *_):
        pass


def verify(compiler, root, release=None):
    root = root.resolve()
    assert not root.exists(), 'Use a fresh QA root.'
    assert registration() is None, 'A previous QA install is still registered.'
    root.mkdir(parents=True)
    channel = Channel()
    server = ThreadingHTTPServer(('127.0.0.1', 0), partial(Handler, channel=channel))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    results = []
    try:
        setup = build_bootstrap(compiler, root / 'setup', test=True,
                                channel_base=f'http://127.0.0.1:{server.server_port}')
        original_hash = hashlib.sha256(setup.read_bytes()).hexdigest()
        app = root / 'installed'

        def install(case, success=True, target=app):
            log = root / (case + '.log')
            proc = subprocess.run([str(setup), '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART',
                                   '/DIR=' + str(target), '/LOG=' + str(log)], timeout=90)
            assert (proc.returncode == 0) == success, (case, proc.returncode, str(log))
            if not success:
                assert (app / '_internal/fixture.txt').read_text() == '1.2.4', case
                assert registration()['DisplayVersion'] == '1.2.4', case
            results.append(case)
            print(json.dumps({'case': case, 'passed': True}), flush=True)

        def uninstall(target):
            subprocess.run([str(target / 'unins000.exe'), '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART'], check=True, timeout=60)
            deadline = time.monotonic() + 30
            while registration() is not None and time.monotonic() < deadline:
                time.sleep(.2)
            assert registration() is None

        channel.release('1.2.3')
        install('first-version')
        assert registration()['DisplayVersion'] == '1.2.3'
        assert (app / '_internal/fixture.txt').read_text() == '1.2.3'
        (app / 'user-note.txt').write_text('preserve this file')
        channel.release('1.2.4')
        install('same-installer-new-version')
        assert registration()['DisplayVersion'] == '1.2.4'
        assert (app / '_internal/fixture.txt').read_text() == '1.2.4'
        assert (app / 'user-note.txt').read_text() == 'preserve this file'
        assert hashlib.sha256(setup.read_bytes()).hexdigest() == original_hash

        manifest = '/releases/latest/download/install.ini'
        for case, old, new in [('invalid-protocol', b'protocol=1', b'protocol=2'),
                               ('invalid-version', b'version=1.2.5', b'version=../../other'),
                               ('invalid-size', b'size=', b'size=-'),
                               ('invalid-hash', b'sha256=', b'sha256=x')]:
            channel.release('1.2.5')
            channel.files[manifest] = channel.files[manifest].replace(old, new)
            install(case, False)
        channel.release('1.2.5')
        channel.files[manifest] = channel.files[manifest].replace(b'sha256=', b'sha256=' + b'0' * 64 + b'\nignored=')
        install('hash-mismatch', False)
        channel.release('1.2.5')
        del channel.files[manifest]
        install('missing-manifest', False)
        channel.release('1.2.5')
        del channel.files[f'/releases/download/v1.2.5/{ASSET}']
        install('missing-payload', False)
        channel.release('1.2.5')
        channel.files[f'/releases/download/v1.2.5/{ASSET}'] = b'broken download'
        install('truncated-payload', False)
        channel.release('1.2.5')
        broken = b'not a zip archive'
        ini = configparser.ConfigParser()
        ini.read_string(channel.files[manifest].decode())
        ini['release']['size'] = str(len(broken))
        ini['release']['sha256'] = hashlib.sha256(broken).hexdigest()
        serialized = io.StringIO()
        ini.write(serialized, space_around_delimiters=False)
        channel.files[manifest] = serialized.getvalue().encode()
        channel.files[f'/releases/download/v1.2.5/{ASSET}'] = broken
        install('invalid-archive-with-valid-hash', False)

        # An open executable must prevent overwrite, never close a running app.
        channel.release('1.2.5')
        kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        kernel.CreateFileW.restype = ctypes.c_void_p
        kernel.CreateFileW.argtypes = [ctypes.c_wchar_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_void_p,
                                      ctypes.c_ulong, ctypes.c_ulong, ctypes.c_void_p]
        kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        handle = kernel.CreateFileW(str(app / 'CodexManager.exe'), 0x80000000, 0, None, 3, 0, None)
        assert handle not in (None, ctypes.c_void_p(-1).value)
        try:
            install('locked-executable', False)
        finally:
            kernel.CloseHandle(handle)
        unrelated = root / 'unrelated'
        unrelated.mkdir()
        (unrelated / 'important.txt').write_text('untouched')
        install('unrelated-folder', False, unrelated)
        assert list(unrelated.iterdir()) == [unrelated / 'important.txt']
        junction = root / 'linked-target'
        # Creation only, never recursive deletion through a junction.
        subprocess.run(['cmd', '/c', 'mklink', '/J', str(junction), str(unrelated)], check=True, capture_output=True)
        try:
            install('linked-folder', False, junction)
            assert list(unrelated.iterdir()) == [unrelated / 'important.txt']
        finally:
            os.rmdir(junction)
        uninstall(app)
        assert (app / 'user-note.txt').read_text() == 'preserve this file'
        assert not (app / '_internal').exists()

        if release:
            release = release.resolve()
            ini = configparser.ConfigParser()
            ini.read(release / 'install.ini')
            version = ini['release']['version']
            channel.files = {manifest: (release / 'install.ini').read_bytes(),
                             f'/releases/download/v{version}/{ASSET}': (release / ASSET).read_bytes()}
            verify_real_setup(setup, root / 'real-application', release)
            results.append('real-startup-update-uninstall')
        report = {'state': 'passed', 'cases': results, 'installer_sha256': original_hash,
                  'installer_bytes': setup.stat().st_size, 'requests': channel.requests}
        (root / 'verification.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
        print(json.dumps(report), flush=True)
    finally:
        server.shutdown()
        server.server_close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--compiler', type=Path, required=True)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--release', type=Path)
    args = parser.parse_args()
    verify(args.compiler, args.root, args.release)
