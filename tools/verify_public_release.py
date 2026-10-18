"""Check anonymous public downloads, fresh portable launch and online installation."""
import argparse
import hashlib
import json
from pathlib import Path
import time
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

from publish_release import validate_release, PAYLOAD_NAMES, INSTALLER_NAME, INSTALLER_TAG
from project_manager.version import VERSION, REPOSITORY, REPOSITORY_URL
from project_manager.updates import check_release

API = 'https://api.github.com/repos/' + REPOSITORY


def read_public(url, target=None):
    # Deliberately no GitHub token: release access must work for ordinary users.
    for attempt in range(4):
        try:
            request = Request(url, headers={'User-Agent': 'Codex-Manager-release-verification', 'Cache-Control': 'no-cache'})
            with urlopen(request, timeout=40) as response:
                if target is None:
                    return response.read(1024 * 1024)
                with target.open('wb') as file:
                    while chunk := response.read(1024 * 1024):
                        file.write(chunk)
            return target
        except (HTTPError, URLError, TimeoutError, ConnectionError) as exc:
            if attempt == 3 or isinstance(exc, HTTPError) and exc.code not in (404, 429, 500, 502, 503, 504):
                raise
            time.sleep(2)


def public_latest():
    return json.loads(read_public(API + '/releases/latest'))


def verify_rollback(previous_tag):
    if previous_tag is None:
        try:
            latest = public_latest()
        except HTTPError as exc:
            if exc.code == 404:
                return
            raise
        # A separate installer channel may be the only remaining release.
        if latest['tag_name'].startswith('v'):
            raise RuntimeError('Failed application is still the public latest release.')
        return
    latest = public_latest()
    metadata = json.loads(read_public(REPOSITORY_URL + '/releases/latest/download/update.json'))
    if latest['tag_name'] != previous_tag or latest['draft'] or latest['prerelease'] or metadata['version'] != previous_tag[1:]:
        raise RuntimeError('Previous public update channel was not restored.')


def verify(output, commit, compiler, expected_release=None, installer_sha=None):
    from build_bootstrap import build_bootstrap, MAX_SIZE
    from verify_portable import verify as verify_portable
    from verify_setup import verify as verify_setup
    output = output.resolve()
    if output.exists():
        raise FileExistsError('Use a fresh public verification directory.')
    output.mkdir(parents=True)
    latest = public_latest()
    tag = 'v' + VERSION
    if (latest['tag_name'] != tag or latest['draft'] or latest['prerelease'] or
            latest['target_commitish'] != commit):
        raise ValueError('Public latest release does not match the published source commit.')
    fixed = json.loads(read_public(API + '/releases/tags/' + INSTALLER_TAG))
    if fixed['draft'] or fixed['prerelease']:
        raise ValueError('Fixed installer channel is not public and stable.')
    for name in PAYLOAD_NAMES:
        read_public(REPOSITORY_URL + '/releases/download/' + tag + '/' + name, output / name)
    metadata = validate_release(output)
    if expected_release and metadata != validate_release(expected_release):
        raise ValueError('Public payload differs from the verified build.')
    # Exercise the stable links used by existing installers and older README links.
    for name in ('install.ini', 'update.json'):
        if read_public(REPOSITORY_URL + '/releases/latest/download/' + name) != (output / name).read_bytes():
            raise ValueError('Latest channel metadata differs from the published payload.')
    setup = read_public(REPOSITORY_URL + '/releases/download/' + INSTALLER_TAG + '/' + INSTALLER_NAME, output / INSTALLER_NAME)
    sha = hashlib.sha256(setup.read_bytes()).hexdigest()
    checksums = read_public(REPOSITORY_URL + '/releases/download/' + INSTALLER_TAG + '/SHA256SUMS.txt').decode().strip().split('  ')
    if checksums != [sha, INSTALLER_NAME] or not 0 < setup.stat().st_size <= MAX_SIZE or installer_sha and sha != installer_sha:
        raise ValueError('Fixed installer was changed or failed its checksum/size validation.')
    alias = read_public(REPOSITORY_URL + '/releases/latest/download/' + INSTALLER_NAME, output / 'compatibility-installer.exe')
    if hashlib.sha256(alias.read_bytes()).hexdigest() != sha:
        raise ValueError('Legacy installer download link changed its contents.')
    update = check_release(current='0.2.0')
    if not update or not update.available or update.version != VERSION:
        raise ValueError('Existing clients do not discover the new release.')
    portable = verify_portable(output, output / 'portable')
    # Same installer source and real public channel, separate QA identity so the
    # test cannot overwrite or unregister an existing user's installation.
    qa_setup = build_bootstrap(compiler, output / 'qa-installer', test=True)
    verify_setup(qa_setup, output / 'online-install', output)
    report = {'state': 'passed', 'version': VERSION, 'source_commit': commit,
              'installer_sha256': sha, 'installer_bytes': setup.stat().st_size,
              'payload_sha256': metadata['sha256'], 'payload_bytes': metadata['size'],
              'anonymous_downloads': True, 'legacy_installer_link_preserved': True,
              'old_client_discovers_update': True, 'portable': portable,
              'public_channel_install_update_uninstall': True}
    (output / 'verification.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps(report), flush=True)
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--compiler', type=Path, required=True)
    args = parser.parse_args()
    verify(args.root, args.commit, args.compiler)
