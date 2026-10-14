"""Publish app payloads separately; create the fixed installer only once."""
import argparse
import configparser
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import zipfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'src'))
from project_manager.version import VERSION, REPOSITORY, ASSET_NAME

INSTALLER_TAG = 'installer-v1'
INSTALLER_NAME = 'Codex-Manager-Setup.exe'
PAYLOAD_NAMES = (ASSET_NAME, 'update.json', 'install.ini', 'SHA256SUMS.txt')


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate_release(release):
    metadata = json.loads((release / 'update.json').read_text(encoding='utf-8'))
    ini = configparser.ConfigParser()
    ini.read(release / 'install.ini', encoding='ascii')
    install = ini['release']
    archive = release / ASSET_NAME
    with zipfile.ZipFile(archive) as z:
        expanded = sum(i.file_size for i in z.infolist())
    expected = {'protocol': '1', 'version': VERSION, 'size': str(archive.stat().st_size),
                'sha256': digest(archive), 'expanded_size': str(expanded)}
    if dict(install) != expected or metadata != {
        'version': VERSION, 'asset': ASSET_NAME, 'size': archive.stat().st_size, 'sha256': digest(archive)
    }:
        raise ValueError('Installer and updater metadata must match the exact release payload.')
    if not 0 < expanded <= 2147483647:
        raise ValueError('Payload exceeds bootstrap protocol 1 size limits.')
    checksums = dict(line.split('  ', 1)[::-1] for line in (release / 'SHA256SUMS.txt').read_text().splitlines())
    if checksums != {name: digest(release / name) for name in PAYLOAD_NAMES[:-1]}:
        raise ValueError('Release checksums do not match.')
    return metadata


def gh(*args):
    return subprocess.run(['gh', *args], text=True, encoding='utf-8', capture_output=True, check=True).stdout


def existing_release(tag):
    try:
        return json.loads(gh('api', f'repos/{REPOSITORY}/releases/tags/{tag}'))
    except subprocess.CalledProcessError as error:
        if 'HTTP 404' in error.stderr:
            return None
        raise RuntimeError(error.stderr) from error


def prepare_draft(tag, title, notes, assets, commit):
    existing = existing_release(tag)
    if existing is not None:
        if not existing['draft'] or existing['target_commitish'] != commit:
            raise ValueError(f'{tag} is already published or belongs to another commit; bump the version.')
        gh('release', 'upload', tag, *map(str, assets), '--clobber', '--repo', REPOSITORY)
    else:
        gh('release', 'create', tag, *map(str, assets), '--draft', '--title', title,
           '--notes-file', str(notes), '--target', commit, '--repo', REPOSITORY)


def prepare_application(release, commit, notes):
    tag = 'v' + VERSION
    existing = existing_release(tag)
    if existing and not existing['draft']:
        if existing['target_commitish'] != commit:
            raise ValueError(f'{tag} belongs to another commit; bump the version.')
        # A retry after publishing the app must still finish publishing the
        # installer. Verify the immutable published payload, never replace it
        # with a new (potentially nondeterministic) build from the same commit.
        published = release.parent / 'published-app'
        published.mkdir(parents=True, exist_ok=True)
        patterns = [arg for name in PAYLOAD_NAMES for arg in ('--pattern', name)]
        gh('release', 'download', tag, '--repo', REPOSITORY, '--dir', str(published), *patterns, '--clobber')
        validate_release(published)
        return False
    prepare_draft(tag, 'Codex Manager ' + VERSION, notes, [release / name for name in PAYLOAD_NAMES], commit)
    return True


def publish(compiler, release, commit):
    from build_bootstrap import build_bootstrap, MAX_SIZE
    if not re.fullmatch('[0-9a-f]{40}', commit):
        raise ValueError('Use an exact source commit SHA.')
    release = release.resolve()
    validate_release(release)
    notes = ROOT / f'docs/release-{VERSION}.md'
    if not notes.is_file():
        raise ValueError('Release notes are required.')
    tag = 'v' + VERSION
    existing_installer = existing_release(INSTALLER_TAG)
    bootstrap_dir = release.parent / INSTALLER_TAG
    bootstrap_dir.mkdir(parents=True, exist_ok=True)
    if existing_installer and not existing_installer['draft']:
        # Never rebuild, replace, or edit the installer during app releases.
        gh('release', 'download', INSTALLER_TAG, '--repo', REPOSITORY, '--dir', str(bootstrap_dir),
           '--pattern', INSTALLER_NAME, '--pattern', 'SHA256SUMS.txt', '--clobber')
        setup = bootstrap_dir / INSTALLER_NAME
        expected = (bootstrap_dir / 'SHA256SUMS.txt').read_text().strip().split('  ')
        if expected != [digest(setup), INSTALLER_NAME] or setup.stat().st_size > MAX_SIZE:
            raise ValueError('Published installer checksum or size is invalid.')
    else:
        setup = build_bootstrap(compiler, bootstrap_dir)
        prepare_draft(INSTALLER_TAG, 'Windows 설치 프로그램 · 항상 최신 버전 설치',
                      ROOT / 'docs/installer-release.md', [setup, bootstrap_dir / 'SHA256SUMS.txt'], commit)
    if prepare_application(release, commit, notes):
        gh('release', 'edit', tag, '--draft=false', '--latest', '--repo', REPOSITORY)
    if not existing_installer or existing_installer['draft']:
        gh('release', 'edit', INSTALLER_TAG, '--draft=false', '--latest=false', '--repo', REPOSITORY)
    print(json.dumps({'version': VERSION, 'installer_reused': bool(existing_installer and not existing_installer['draft']),
                      'installer_sha256': digest(setup), 'installer_bytes': setup.stat().st_size}), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--compiler', type=Path, required=True)
    parser.add_argument('--release', type=Path, required=True)
    parser.add_argument('--commit', required=True)
    args = parser.parse_args()
    publish(args.compiler, args.release, args.commit)
