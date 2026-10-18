"""Validate the complete portable payload before it can be published/extracted."""
import hashlib
import json
import re
import stat
import sys
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))
from project_manager.updates import checked_relative
from project_manager.version import EXECUTABLE


def public_path(name):
    checked_relative(name)
    parts = name.lower().split('/')
    private_names = {'auth.json', '.codex-global-state.json', 'settings.json',
                     'id_rsa', 'id_ed25519', '.git', '.codex', 'sessions', 'archived_sessions'}
    if any(p in private_names or p == '.env' or p.startswith('.env.') or
           p.endswith(('.sqlite', '.sqlite3', '.db', '.jsonl', '.sqlite-wal', '.sqlite-shm'))
           for p in parts):
        raise ValueError('Private data path is not allowed in the application payload.')
    if parts[-1].startswith('unins') and parts[-1].endswith(('.exe', '.dat')):
        raise ValueError('The portable payload must not contain an installed uninstaller.')
    return name


def validate_archive(archive, version):
    with zipfile.ZipFile(archive) as z:
        entries = {}
        folded = set()
        for entry in z.infolist():
            if not entry.filename.startswith('CodexManager/') or entry.is_dir():
                raise ValueError('Unexpected payload root or directory entry.')
            name = public_path(entry.filename[len('CodexManager/'):])
            if name.casefold() in folded or stat.S_ISLNK(entry.external_attr >> 16) or entry.external_attr & 0x400:
                raise ValueError('Duplicate or linked payload path.')
            folded.add(name.casefold())
            entries[name] = entry
        if sum(e.file_size for e in entries.values()) > 2147483647:
            raise ValueError('Payload exceeds bootstrap protocol 1 size limits.')
        manifest_entry = entries.get('install-manifest.json')
        if not manifest_entry or manifest_entry.file_size > 512 * 1024:
            raise ValueError('Missing or oversized payload manifest.')
        manifest = json.loads(z.read(manifest_entry))
        expected = manifest.get('files')
        if (manifest.get('version') != version or not isinstance(expected, dict) or
                set(entries) != set(expected) | {'install-manifest.json'} or
                EXECUTABLE not in expected or not any(n.startswith('_internal/') for n in expected)):
            raise ValueError('Payload manifest does not describe the complete application.')
        for name, sha in expected.items():
            if not isinstance(sha, str) or not re.fullmatch('[0-9a-f]{64}', sha):
                raise ValueError('Invalid payload file digest.')
            hasher = hashlib.sha256()
            with z.open(entries[name]) as file:
                while chunk := file.read(1024 * 1024):
                    hasher.update(chunk)
            if hasher.hexdigest() != sha:
                raise ValueError('Payload file checksum mismatch: ' + name)
        return manifest
