"""Build the fixed online installer independently of application releases."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
MAX_SIZE = 5 * 1024 * 1024


def build_bootstrap(compiler, output, test=False, channel_base=None):
    if channel_base is not None:
        if not test:
            raise ValueError('Channel override is only allowed for QA builds.')
        if not re.fullmatch(r'http://127\.0\.0\.1:[1-9][0-9]{0,4}', channel_base):
            raise ValueError('QA channel must be a loopback origin with an explicit port.')
        if int(channel_base.rsplit(':', 1)[1]) > 65535:
            raise ValueError('QA loopback port is invalid.')
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    args = [str(compiler.resolve()), '/Qp', '/DOutputDir=' + str(output),
            '/DIconPath=' + str(ROOT / 'src/project_manager/ui/app-icon.ico')]
    if test:
        args += ['/DQA=1']
        if channel_base:
            args += ['/DChannelBase=' + channel_base]
    subprocess.run(args + [str(ROOT / 'packaging/bootstrap.iss')], check=True)
    setup = output / 'Codex-Manager-Setup.exe'
    size = setup.stat().st_size
    if size > MAX_SIZE:
        raise ValueError(f'Online installer exceeds 5 MiB: {size}')
    sha256 = hashlib.sha256(setup.read_bytes()).hexdigest()
    (output / 'SHA256SUMS.txt').write_text(sha256 + '  ' + setup.name + '\n', encoding='ascii')
    if test:
        (output / 'setup-qa.json').write_text(json.dumps({'test': True, 'sha256': sha256}), encoding='utf-8')
    print(json.dumps({'setup': str(setup), 'size': size, 'sha256': sha256, 'test': test}), flush=True)
    return setup


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--compiler', type=Path, required=True)
    parser.add_argument('--output', type=Path, default=ROOT / 'releases/installer-v1')
    parser.add_argument('--test', action='store_true')
    parser.add_argument('--channel-base')
    args = parser.parse_args()
    build_bootstrap(args.compiler, args.output, args.test, args.channel_base)
