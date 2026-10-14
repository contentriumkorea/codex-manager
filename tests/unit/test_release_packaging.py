"""Installer and updater must consume the exact same published payload."""
import configparser
import hashlib
import importlib.util
import json
from pathlib import Path
import zipfile

import pytest

ROOT = Path(__file__).resolve().parents[2]


def load_tool(name):
    path = ROOT / 'tools' / (name + '.py')
    assert path.is_file(), f'Missing standalone tool: {name}'
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_payload_release_has_consistent_online_install_metadata(tmp_path):
    dist = tmp_path / 'dist'
    (dist / '_internal').mkdir(parents=True)
    (dist / 'CodexManager.exe').write_bytes(b'fixture executable')
    (dist / '_internal' / 'library.dll').write_bytes(b'fixture library')
    output = tmp_path / 'release'
    archive = load_tool('build_release').build(dist, output)
    assert (output / 'install.ini').is_file(), 'Online installer needs its own channel metadata'
    ini = configparser.ConfigParser()
    ini.read(output / 'install.ini', encoding='utf-8')
    install = ini['release']
    update = json.loads((output / 'update.json').read_text())
    assert install['protocol'] == '1'
    assert install['version'] == update['version']
    assert install.getint('size') == update['size'] == archive.stat().st_size
    assert install['sha256'] == update['sha256'] == hashlib.sha256(archive.read_bytes()).hexdigest()
    with zipfile.ZipFile(archive) as z:
        assert install.getint('expanded_size') == sum(i.file_size for i in z.infolist())
        assert 'CodexManager/install-manifest.json' in z.namelist()
    checksums = dict(line.split('  ', 1)[::-1] for line in (output / 'SHA256SUMS.txt').read_text().splitlines())
    assert set(checksums) == {archive.name, 'update.json', 'install.ini'}
    for name, expected in checksums.items():
        assert hashlib.sha256((output / name).read_bytes()).hexdigest() == expected
    assert not list(output.glob('*.exe')), 'App releases must not rebuild the bootstrap'


def test_production_bootstrap_cannot_override_download_origin(tmp_path):
    tool = load_tool('build_bootstrap')
    with pytest.raises(ValueError, match='QA'):
        tool.build_bootstrap(Path('not-a-compiler'), tmp_path, channel_base='https://example.com')


@pytest.mark.parametrize('base', ['https://example.com', 'http://localhost:8000', 'http://127.0.0.1:8000/path', 'http://127.0.0.1:8000/\"bad'])
def test_qa_override_only_accepts_loopback_origin(tmp_path, base):
    with pytest.raises(ValueError, match='loopback'):
        load_tool('build_bootstrap').build_bootstrap(Path('not-a-compiler'), tmp_path, test=True, channel_base=base)


def test_publisher_rejects_mismatched_installer_and_update_metadata(tmp_path):
    dist = tmp_path / 'dist'
    (dist / '_internal').mkdir(parents=True)
    (dist / 'CodexManager.exe').write_bytes(b'fixture')
    (dist / '_internal/library').write_bytes(b'library')
    output = tmp_path / 'release'
    load_tool('build_release').build(dist, output)
    publisher = load_tool('publish_release')
    assert publisher.validate_release(output)['version']
    ini = output / 'install.ini'
    ini.write_text(ini.read_text().replace('protocol=1', 'protocol=2'))
    with pytest.raises(ValueError):
        publisher.validate_release(output)


def test_publication_can_resume_after_app_is_public_but_installer_is_draft(tmp_path, monkeypatch):
    publisher = load_tool('publish_release')
    commit = 'a' * 40
    existing = {'draft': False, 'target_commitish': commit}
    monkeypatch.setattr(publisher, 'existing_release', lambda tag: existing)
    commands = []
    monkeypatch.setattr(publisher, 'gh', lambda *args: commands.append(args))
    validated = []
    monkeypatch.setattr(publisher, 'validate_release', lambda path: validated.append(path))
    assert hasattr(publisher, 'prepare_application'), 'Publication needs a safe resume path'
    assert publisher.prepare_application(tmp_path / 'app', commit, tmp_path / 'notes.md') is False
    assert len(validated) == 1
    assert commands[0][:2] == ('release', 'download')
    assert all('upload' not in c and 'edit' not in c for c in commands), 'Published app must remain immutable'
    existing['target_commitish'] = 'b' * 40
    with pytest.raises(ValueError, match='commit'):
        publisher.prepare_application(tmp_path / 'app', commit, tmp_path / 'notes.md')
