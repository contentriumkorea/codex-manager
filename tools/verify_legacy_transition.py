"""Exercise the updater code embedded in the actual published 0.10.4 executable.

The unmodified frozen launcher/helper run in an isolated fixture; this does not
claim that the old Qt button was operated or that a second physical PC was used.
"""
import argparse
import hashlib
import importlib.abc
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import zipfile
from PyInstaller.archive.readers import CArchiveReader
from release_payload import validate_archive


def verify(previous, release, root):
    previous, release, root = previous.resolve(), release.resolve(), root.resolve()
    old_sha = '67e1979947aa11d7b58e39a779f1eb83eb5502e254e51da41ea4f35f1b781324'
    if root.exists() or hashlib.sha256(previous.read_bytes()).hexdigest() != old_sha:
        raise ValueError('Use the exact published 0.10.4 payload and a fresh QA root.')
    validate_archive(previous, '0.10.4')
    root.mkdir(parents=True)
    with zipfile.ZipFile(previous) as archive:
        archive.extractall(root)
    app = root / 'CodexManager'
    old = CArchiveReader(str(app / 'CodexManager.exe')).open_embedded_archive('PYZ.pyz')
    package_root = app / '_internal/project_manager'

    class FrozenLoader(importlib.abc.MetaPathFinder, importlib.abc.Loader):
        def find_spec(self, fullname, path=None, target=None):
            if fullname != 'legacy0104' and not fullname.startswith('legacy0104.'):
                return None
            original = fullname.replace('legacy0104', 'project_manager', 1)
            if original not in old.toc:
                return None
            return importlib.util.spec_from_loader(fullname, self, is_package=old.toc[original][0] == 1)

        def create_module(self, spec):
            return None

        def exec_module(self, module):
            suffix = module.__name__.split('.')[1:]
            module.__file__ = str(package_root.joinpath(*suffix).with_suffix('.py')) if suffix else str(package_root / '__init__.py')
            exec(old.extract(module.__name__.replace('legacy0104', 'project_manager', 1)), module.__dict__)

    loader = FrozenLoader()
    sys.meta_path.insert(0, loader)
    import legacy0104.updates as updater
    assert updater.VERSION == '0.10.4'
    state, home = root / 'state', root / 'home'
    state.mkdir(); home.mkdir()
    settings = {'home': str(home), 'startup_update_check': False, 'legacy_fixture': 'preserve'}
    (state / 'settings.json').write_text(json.dumps(settings), encoding='utf-8')
    (app / 'personal-note.txt').write_text('preserve unrelated file', encoding='utf-8')
    metadata = json.loads((release / 'update.json').read_text())
    candidate = updater.Release(metadata['version'], True, '', '', '', '')
    plan = updater.stage_update(release / updater.ASSET_NAME, metadata, candidate, app, state, parent_pid=2147483647)
    # Run exactly the old frozen function and helper with its original 20s check.
    os.environ['PSModulePath'] = str(Path(os.environ['SystemRoot']) / 'System32/WindowsPowerShell/v1.0/Modules')
    started = time.monotonic()
    sys.frozen = True
    try:
        updater.launch_installer(plan)
    finally:
        sys.frozen = False
    ready_seconds = time.monotonic() - started
    deadline = time.monotonic() + 90
    result = Path(plan['result'])
    while not result.exists() and time.monotonic() < deadline:
        time.sleep(.1)
    record = json.loads(result.read_text())
    if record.get('state') != 'completed':
        raise RuntimeError(record)
    health = json.loads(Path(plan['health']).read_text())
    try:
        assert ready_seconds < 20
        assert health['version'] == candidate.version and Path(health['install_dir']).resolve() == app
        assert health['taskbar_registered'] and not health['taskbar_error']
        assert (Path(plan['backup']) / 'personal-note.txt').read_text() == 'preserve unrelated file'
        actual = json.loads((state / 'settings.json').read_text())
        assert all(actual.get(key) == value for key, value in settings.items())
        report = {'state': 'passed', 'legacy_frozen_version': updater.VERSION, 'legacy_payload_sha256': old_sha,
                  'version': candidate.version, 'ready_seconds': round(ready_seconds, 3),
                  'total_seconds': round(time.monotonic() - started, 3), 'health': health, 'result': record}
        (root / 'verification.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
        print(json.dumps(report))
    finally:
        pid = int(health['pid'])
        subprocess.run(['powershell.exe', '-NoProfile', '-NonInteractive', '-Command',
                        f'$p=Get-Process -Id {pid} -ErrorAction Stop;$null=$p.CloseMainWindow();$null=$p.WaitForExit(20000)'],
                       check=True, capture_output=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--previous', type=Path, required=True)
    parser.add_argument('--release', type=Path, required=True)
    parser.add_argument('--root', type=Path, required=True)
    args = parser.parse_args()
    verify(args.previous, args.release, args.root)
