"""Build the native application and the existing protocol-1 release assets."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import pefile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
from build_release import build


def run(*args):
    env = os.environ.copy()
    if args[0].lower() == 'powershell.exe':
        env = {key: value for key, value in env.items() if key.upper() != 'PSMODULEPATH'}
        env['PSMODULEPATH'] = str(Path(os.environ['SystemRoot']) / 'System32/WindowsPowerShell/v1.0/Modules')
    subprocess.run(args, cwd=ROOT, env=env, check=True)


def tree_hashes(root):
    result = {}
    for path in sorted(root.rglob('*')):
        if path.is_symlink() or path.stat().st_file_attributes & 0x400:
            raise ValueError('Linked runtime content is not allowed.')
        if path.is_file():
            hasher = hashlib.sha256()
            with path.open('rb') as file:
                while chunk := file.read(1024 * 1024):
                    hasher.update(chunk)
            result[path.relative_to(root).as_posix()] = hasher.hexdigest()
    return result


def stage(runtime, cab, output):
    runtime, output = runtime.resolve(), output.resolve()
    if output.exists():
        raise FileExistsError('Use a new build output directory.')
    cab_sha = '857dcfd6f7035508a29645fa2978c3a6a841f220df77cc366d318894e19f4792'
    if hashlib.sha256(cab.read_bytes()).hexdigest() != cab_sha:
        raise ValueError('The original Microsoft CAB checksum does not match.')
    version = json.loads((ROOT / 'rust-desktop/tauri.conf.json').read_text())['version']
    flags = os.environ.get('CARGO_ENCODED_RUSTFLAGS', '').split('\x1f')
    flags = [flag for flag in flags if flag]
    if 'target-feature=+crt-static' not in flags:
        flags.extend(['-C', 'target-feature=+crt-static'])
    for path, label in [(ROOT, '.'), (Path(os.environ['USERPROFILE']), '<build-user>')]:
        flags.append('--remap-path-prefix=' + str(path) + '=' + label)
    os.environ['CARGO_ENCODED_RUSTFLAGS'] = '\x1f'.join(flags)
    run('cargo', 'build', '--release', '--locked', '--manifest-path', 'rust-desktop/Cargo.toml')
    target = Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'rust-desktop/target')).resolve()
    executable = target / 'release/codex-manager.exe'
    if not executable.is_file() or not (runtime / 'msedgewebview2.exe').is_file():
        raise ValueError('Native executable or complete fixed runtime is missing.')
    reported = subprocess.check_output([str(executable), '--version'], cwd=ROOT, timeout=15,
                                       creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0)).decode('utf-8').strip()
    if reported != 'Codex Manager ' + version:
        raise ValueError('Native runtime version differs from release metadata.')
    binary = executable.read_bytes()
    if any(str(path).encode(encoding) in binary for path in (ROOT, Path(os.environ['USERPROFILE'])) for encoding in ('utf-8', 'utf-16-le')):
        raise ValueError('A private build path remains in the application binary.')
    image = pefile.PE(data=binary)
    imported = [entry.dll.decode('ascii').lower() for name in ('DIRECTORY_ENTRY_IMPORT', 'DIRECTORY_ENTRY_DELAY_IMPORT') for entry in getattr(image, name, [])]
    if any(name.startswith(('vcruntime', 'msvcp', 'concrt')) for name in imported):
        raise ValueError('The application unexpectedly requires a separately installed VC runtime.')
    # Verify the Microsoft executable before redistributing the untouched runtime.
    run('powershell.exe', '-NoProfile', '-NonInteractive', '-File', str(ROOT / 'tools/verify_microsoft_runtime.ps1'), '-Path', str(runtime / 'msedgewebview2.exe'))
    verified = output / 'runtime-origin'
    verified.mkdir(parents=True)
    subprocess.run(['expand.exe', str(cab.resolve()), '-F:*', str(verified)], check=True, capture_output=True)
    trusted_tree = tree_hashes(verified / 'Microsoft.WebView2.FixedVersionRuntime.156.0.4314.58.x64')
    if tree_hashes(runtime) != trusted_tree:
        raise ValueError('The complete runtime differs from the verified Microsoft CAB.')
    app = output / 'dist/CodexManager'
    internal = app / '_internal'
    internal.mkdir(parents=True)
    shutil.copy2(executable, app / 'CodexManager.exe')
    icon = internal / 'project_manager/ui/app-icon.ico'
    icon.parent.mkdir(parents=True)
    shutil.copy2(ROOT / 'rust-desktop/icons/icon.ico', icon)
    shutil.copytree(runtime, internal / 'WebView2')
    if tree_hashes(internal / 'WebView2') != trusted_tree:
        raise ValueError('Runtime bytes changed during payload staging.')
    shutil.copy2(ROOT / 'rust-desktop/ui/licenses/webview2-eula.html', internal / 'webview2-eula.html')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version', '1', '--manifest-path', 'rust-desktop/Cargo.toml'], cwd=ROOT, text=True, encoding='utf-8'))
    notices = ['Codex Manager third-party software notices\n',
               'Microsoft Edge WebView2 Runtime 156.0.4314.58: see webview2-eula.html.\n',
               'The complete Microsoft runtime and its embedded credits are retained under WebView2/.\n']
    for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
        if package['name'] in ('codex-manager', 'codex-manager-core'):
            continue
        folder = Path(package['manifest_path']).parent
        notices.append(f"\n{'=' * 72}\n{package['name']} {package['version']} — {package.get('license') or 'See accompanying license'}\n")
        files = sorted({p for p in folder.iterdir() if p.is_file() and p.name.lower().startswith(('license', 'licence', 'copying', 'notice'))})
        if package.get('license_file'):
            files = sorted(set(files) | {folder / package['license_file']})
        for file in files:
            notices.append(f'\n--- {file.name} ---\n' + file.read_text(encoding='utf-8', errors='replace') + '\n')
    (internal / 'THIRD-PARTY-NOTICES.txt').write_text(''.join(notices), encoding='utf-8')
    (internal / 'runtime-version.json').write_text(json.dumps({'version': '156.0.4314.58', 'cab_sha256': cab_sha, 'tree_sha256': hashlib.sha256(json.dumps(trusted_tree, sort_keys=True).encode()).hexdigest()}), encoding='utf-8')
    archive = build(app, output / 'app')
    print(json.dumps({'version': version, 'archive': str(archive), 'size': archive.stat().st_size}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--runtime', type=Path, required=True)
    parser.add_argument('--cab', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    stage(args.runtime, args.cab, args.output)
