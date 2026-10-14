"""Keep the desktop's persistent sidebar catalog in sync with local deletions.

The CLI's thread/delete cannot notify a closed Electron app. Its separate
catalog must receive the same removal and paused-scan tombstone as the app's
applyAuthoritativeRemoval. Never touch ChatGPT or other hosts' entries.
"""
import sqlite3
import sys
from contextlib import ExitStack, closing


_COLUMNS = {
    'local_thread_catalog': {'host_id', 'thread_id', 'source_kind', 'missing_candidate'},
    'local_thread_catalog_hosts': {'host_id', 'host_kind'},
    'local_thread_catalog_sync_state': {'host_id', 'observation_sequence'},
    'local_thread_catalog_metadata': {'id', 'catalog_revision'},
    'local_thread_catalog_scan_checkpoints': {'host_id'},
    'local_thread_catalog_scan_entries': {'host_id', 'thread_id', 'removed'},
}


class DesktopCatalogDeletion:
    """Lock and validate existing catalogs before the first destructive RPC."""

    def __init__(self, home):
        self.home = home
        self.stack = ExitStack()
        self.databases = []

    def __enter__(self):
        try:
            for name in ('codex.db', 'codex-dev.db'):
                path = self.home / 'sqlite' / name
                if not path.exists():
                    continue
                db = self.stack.enter_context(closing(sqlite3.connect(path.as_uri() + '?mode=rw', uri=True, timeout=2)))
                self.stack.enter_context(db)
                db.execute('BEGIN IMMEDIATE')
                tables = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
                if 'local_thread_catalog' not in tables:
                    continue
                for table, required in _COLUMNS.items():
                    columns = {r[1] for r in db.execute(f'PRAGMA table_info({table})')}
                    if not required <= columns:
                        raise RuntimeError('Codex의 대화 목록 형식이 변경되어 삭제를 중단했습니다. Codex Manager 업데이트가 필요합니다.')
                host = db.execute('SELECT host_kind FROM local_thread_catalog_hosts WHERE host_id=?', ('local',)).fetchone()
                if host is None:
                    continue
                if host != ('local',):
                    raise RuntimeError('Codex의 로컬 대화 목록을 확인할 수 없어 삭제를 중단했습니다.')
                if db.execute('SELECT 1 FROM local_thread_catalog_sync_state WHERE host_id=?', ('local',)).fetchone() is None:
                    raise RuntimeError('Codex의 대화 목록 갱신 정보를 확인할 수 없어 삭제를 중단했습니다.')
                if db.execute('SELECT 1 FROM local_thread_catalog_metadata WHERE id=1').fetchone() is None:
                    raise RuntimeError('Codex의 대화 목록 버전을 확인할 수 없어 삭제를 중단했습니다.')
                self.databases.append(db)
            return self
        except BaseException:
            self.stack.__exit__(*sys.exc_info())
            raise

    def remove(self, thread_ids):
        for db in self.databases:
            changed = False
            checkpoint = db.execute('SELECT 1 FROM local_thread_catalog_scan_checkpoints WHERE host_id=?', ('local',)).fetchone()
            for tid in sorted(set(thread_ids)):
                entry = db.execute('SELECT source_kind, missing_candidate FROM local_thread_catalog WHERE host_id=? AND thread_id=?', ('local', tid)).fetchone()
                if entry and entry[0] == 'chatgpt':
                    continue
                db.execute('UPDATE local_thread_catalog_sync_state SET observation_sequence=observation_sequence+1 WHERE host_id=?', ('local',))
                if checkpoint:
                    db.execute('''INSERT INTO local_thread_catalog_scan_entries (host_id,thread_id,removed)
                        VALUES (?,?,1) ON CONFLICT(host_id,thread_id) DO UPDATE SET removed=1''', ('local', tid))
                db.execute('DELETE FROM local_thread_catalog WHERE host_id=? AND thread_id=?', ('local', tid))
                changed = changed or bool(entry and entry[1] == 0)
            if changed:
                db.execute('UPDATE local_thread_catalog_metadata SET catalog_revision=catalog_revision+1 WHERE id=1')

    def __exit__(self, *exc):
        return self.stack.__exit__(*exc)
