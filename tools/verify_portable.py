"""Launch a fresh portable ZIP with isolated state and no installed runtime on PATH."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import winreg
import zipfile

from publish_release import validate_release
from project_manager.version import VERSION, ASSET_NAME


def registry_values(root, path):
    try:
        with winreg.OpenKey(root, path) as key:
            values = []
            for i in range(winreg.QueryInfoKey(key)[1]):
                values.append(winreg.EnumValue(key, i))
            return sorted(values)
    except FileNotFoundError:
        return []


def registry_children(root, path):
    try:
        with winreg.OpenKey(root, path) as key:
            return sorted(winreg.EnumKey(key, i) for i in range(winreg.QueryInfoKey(key)[0]))
    except FileNotFoundError:
        return []


def system_snapshot():
    uninstall = r'Software\Microsoft\Windows\CurrentVersion\Uninstall'
    result = {}
    for hive, name in ((winreg.HKEY_CURRENT_USER, 'user'), (winreg.HKEY_LOCAL_MACHINE, 'machine')):
        children = registry_children(hive, uninstall)
        result[name + '_uninstall'] = {child: registry_values(hive, uninstall + '\\' + child)
                                     for child in children if 'codexmanager' in child.lower()}
        for key in ('Run', 'RunOnce'):
            result[name + '_' + key] = registry_values(hive, r'Software\Microsoft\Windows\CurrentVersion' + '\\' + key)
    result['services'] = registry_children(winreg.HKEY_LOCAL_MACHINE, r'SYSTEM\CurrentControlSet\Services')
    # Use the actual redirected Desktop (for example OneDrive), not an assumed path.
    shell = dict((n, v) for n, v, _ in registry_values(winreg.HKEY_CURRENT_USER,
                 r'Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders'))
    folders = [Path(os.environ['APPDATA']) / 'Microsoft/Windows/Start Menu/Programs',
               Path(os.environ['PROGRAMDATA']) / 'Microsoft/Windows/Start Menu/Programs',
               Path(os.path.expandvars(shell.get('Desktop', str(Path.home() / 'Desktop')))),
               Path(os.environ.get('PUBLIC', r'C:\Users\Public')) / 'Desktop']
    from project_manager.files import digest
    result['shortcuts'] = {str(p): digest(p) for folder in folders if folder.exists()
                           for p in folder.rglob('*.lnk') if 'codex' in p.name.lower()}
    return result


def verify(release, root):
    release, root = release.resolve(), root.resolve()
    if root.exists():
        raise FileExistsError('Use a fresh portable verification directory.')
    validate_release(release)
    root.mkdir(parents=True)
    with zipfile.ZipFile(release / ASSET_NAME) as z:
        z.extractall(root / 'extracted')  # All paths and hashes were validated above.
    app = root / 'extracted/CodexManager'
    state, home = root / 'state', root / 'home'
    state.mkdir(); home.mkdir()
    settings = {'home': str(home), 'startup_update_check': False, 'portable_test': 'retained'}
    (state / 'settings.json').write_text(json.dumps(settings), encoding='utf-8')
    health = state / 'updates/portable-health.json'
    health.parent.mkdir()
    before = system_snapshot()
    env = os.environ.copy()
    env.pop('QT_QPA_PLATFORM', None)
    windows = Path(os.environ['SystemRoot'])
    env['PATH'] = os.pathsep.join([str(windows), str(windows / 'System32'),
                                  str(windows / 'System32/WindowsPowerShell/v1.0')])
    process = subprocess.Popen([str(app / 'CodexManager.exe'), '--home', str(home), '--state-dir', str(state),
                                '--no-update-check', '--update-health', str(health)],
                               cwd=root, env=env, creationflags=subprocess.CREATE_NO_WINDOW)
    try:
        deadline = time.monotonic() + 60
        while not health.exists():
            if process.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError('Fresh portable application did not start.')
            time.sleep(.2)
        record = json.loads(health.read_text(encoding='utf-8'))
        assert record['version'] == VERSION and Path(record['install_dir']).resolve() == app
        assert record['pid'] == process.pid and record['taskbar_registered'] and not record['taskbar_error']
        assert process.poll() is None
        # Only close this child process; never interact with the user's app.
        script = f'$p=Get-Process -Id {process.pid} -ErrorAction Stop; $null=$p.CloseMainWindow()'
        subprocess.run(['powershell.exe', '-NoProfile', '-Command', script], check=True,
                       capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW)
        process.wait(timeout=30)
        assert process.returncode == 0
    finally:
        if process.poll() is None:
            process.terminate(); process.wait(timeout=15)
    assert system_snapshot() == before, 'Portable launch changed permanent installation registration.'
    assert not list(app.glob('unins*'))
    current = json.loads((state / 'settings.json').read_text(encoding='utf-8'))
    assert all(current.get(k) == v for k, v in settings.items())
    report = {'state': 'passed', 'version': VERSION, 'fresh_zip_direct_launch': True,
              'taskbar_registered': True, 'permanent_registration_unchanged': True,
              'installed_runtime_removed_from_path': True, 'isolated_settings_retained': True}
    (root / 'verification.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps(report), flush=True)
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--release', type=Path, required=True)
    parser.add_argument('--root', type=Path, required=True)
    args = parser.parse_args()
    verify(args.release, args.root)
