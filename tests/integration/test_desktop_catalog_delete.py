import sqlite3
from types import SimpleNamespace

import pytest

from project_manager.codex.adapter import CodexAdapter
from test_operations import make_project


def seed_catalog(home, name, thread_id, other_id):
    path = home / 'sqlite' / name
    path.parent.mkdir(exist_ok=True)
    with sqlite3.connect(path) as db:
        db.executescript('''
            CREATE TABLE local_thread_catalog (
                host_id TEXT, thread_id TEXT, source_kind TEXT,
                display_title TEXT, missing_candidate INTEGER DEFAULT 0,
                PRIMARY KEY (host_id, thread_id));
            CREATE TABLE local_thread_catalog_hosts (host_id TEXT PRIMARY KEY, host_kind TEXT);
            CREATE TABLE local_thread_catalog_sync_state (host_id TEXT PRIMARY KEY, observation_sequence INTEGER);
            CREATE TABLE local_thread_catalog_metadata (id INTEGER PRIMARY KEY, catalog_revision INTEGER);
            CREATE TABLE local_thread_catalog_scan_checkpoints (host_id TEXT PRIMARY KEY, checkpoint TEXT);
            CREATE TABLE local_thread_catalog_scan_entries (
                host_id TEXT, thread_id TEXT, removed INTEGER, PRIMARY KEY (host_id, thread_id));
            INSERT INTO local_thread_catalog_hosts VALUES ('local', 'local'), ('remote', 'ssh'), ('cloud', 'chatgpt');
            INSERT INTO local_thread_catalog_sync_state VALUES ('local', 7), ('remote', 8), ('cloud', 9);
            INSERT INTO local_thread_catalog_metadata VALUES (1, 12);
            INSERT INTO local_thread_catalog_scan_checkpoints VALUES ('local', 'paused scan');
        ''')
        db.executemany('INSERT INTO local_thread_catalog VALUES (?,?,?,?,0)', [
            ('local', thread_id, 'vscode', 'Deleted task'),
            ('local', other_id, 'vscode', 'Keep this task'),
            ('remote', thread_id, 'vscode', 'Remote task'),
            ('cloud', thread_id, 'chatgpt', 'Cloud chat'),
        ])
    return path


@pytest.mark.parametrize('name', ['codex.db', 'codex-dev.db'])
def test_delete_removes_sidebar_title_before_final_ui_sync(tmp_path, name):
    adapter = CodexAdapter(tmp_path / 'home', isolated=True)
    _, tid = make_project(adapter, tmp_path / 'A', 'A')
    _, other = make_project(adapter, tmp_path / 'B', 'B')
    path = seed_catalog(adapter.home, name, tid, other)

    adapter.delete_thread(tid)

    assert tid not in {t.id for t in adapter.snapshot().conversations}
    with sqlite3.connect(path) as db:
        assert db.execute('SELECT 1 FROM local_thread_catalog WHERE host_id=? AND thread_id=?', ('local', tid)).fetchone() is None
        assert db.execute('SELECT COUNT(*) FROM local_thread_catalog').fetchone()[0] == 3
        assert db.execute('SELECT removed FROM local_thread_catalog_scan_entries WHERE host_id=? AND thread_id=?', ('local', tid)).fetchone() == (1,)
        assert db.execute('SELECT observation_sequence FROM local_thread_catalog_sync_state WHERE host_id="local"').fetchone() == (8,)
        assert db.execute('SELECT catalog_revision FROM local_thread_catalog_metadata').fetchone() == (13,)
    assert adapter.read_thread(other)['id'] == other


def fake_adapter(tmp_path, monkeypatch):
    adapter = CodexAdapter(tmp_path / 'home', isolated=True)
    adapter.home.mkdir()
    threads = {'parent': None, 'child': 'parent', 'keep': None}
    monkeypatch.setattr(adapter, 'ensure_write_allowed', lambda: None)
    monkeypatch.setattr(adapter, 'snapshot', lambda **kwargs: SimpleNamespace(
        conversations=[SimpleNamespace(id=tid, parent_id=parent) for tid, parent in threads.items()]))
    return adapter, threads


@pytest.mark.parametrize('partial', [False, True])
def test_failed_rpc_only_cleans_confirmed_deletions(tmp_path, monkeypatch, partial):
    adapter, threads = fake_adapter(tmp_path, monkeypatch)
    path = seed_catalog(adapter.home, 'codex.db', 'parent', 'keep')

    def fail(*args):
        if partial:
            del threads['parent']
        raise RuntimeError('RPC interrupted')

    monkeypatch.setattr(adapter, 'call', fail)
    with pytest.raises(RuntimeError, match='RPC interrupted'):
        adapter.delete_thread('parent')
    with sqlite3.connect(path) as db:
        exists = db.execute('SELECT 1 FROM local_thread_catalog WHERE host_id="local" AND thread_id="parent"').fetchone()
        assert bool(exists) == (not partial)
        assert db.execute('SELECT COUNT(*) FROM local_thread_catalog').fetchone()[0] == (3 if partial else 4)


def test_delete_cleans_descendants_and_both_desktop_flavors(tmp_path, monkeypatch):
    adapter, threads = fake_adapter(tmp_path, monkeypatch)
    paths = [seed_catalog(adapter.home, name, 'parent', 'keep') for name in ('codex.db', 'codex-dev.db')]
    for path in paths:
        with sqlite3.connect(path) as db:
            db.execute('INSERT INTO local_thread_catalog VALUES (?,?,?,?,0)', ('local', 'child', 'vscode', 'Child title'))

    def delete(*args):
        del threads['parent'], threads['child']

    monkeypatch.setattr(adapter, 'call', delete)
    adapter.delete_thread('parent')
    for path in paths:
        with sqlite3.connect(path) as db:
            assert db.execute('SELECT thread_id FROM local_thread_catalog WHERE host_id="local"').fetchall() == [('keep',)]
            assert db.execute('SELECT COUNT(*) FROM local_thread_catalog_scan_entries WHERE removed=1').fetchone() == (2,)


@pytest.mark.parametrize('blocked', ['schema', 'locked'])
def test_catalog_preflight_prevents_destructive_rpc(tmp_path, monkeypatch, blocked):
    adapter, threads = fake_adapter(tmp_path, monkeypatch)
    path = seed_catalog(adapter.home, 'codex.db', 'parent', 'keep')
    calls = []
    monkeypatch.setattr(adapter, 'call', lambda *args: calls.append(args))
    db = sqlite3.connect(path)
    try:
        if blocked == 'schema':
            db.execute('DROP TABLE local_thread_catalog_scan_entries')
            db.commit()
        else:
            db.execute('BEGIN IMMEDIATE')
        with pytest.raises((RuntimeError, sqlite3.OperationalError)):
            adapter.delete_thread('parent')
        assert not calls
        assert set(threads) == {'parent', 'child', 'keep'}
    finally:
        db.close()
