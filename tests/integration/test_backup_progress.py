from project_manager.bundles import export_project, verify_bundle
from project_manager.codex.adapter import CodexAdapter
from project_manager.progress import ProgressReporter
from project_manager.journal import Journal
from project_manager.ui.batch import run_backups
from test_operations import make_project


def test_backup_keeps_reporting_after_file_copy_hits_100_percent(tmp_path):
    adapter = CodexAdapter(tmp_path / 'home', isolated=True)
    project, _ = make_project(adapter, tmp_path / 'project', 'A')
    (project.roots[0] / 'large.bin').write_bytes(b'x' * (3 * 1024 * 1024))
    events = []
    progress = ProgressReporter(events.append, interval=0)
    destination = tmp_path / 'backup'
    assert export_project(project, adapter.snapshot(), destination, progress).ok
    copied = next(i for i, e in enumerate(events) if e.phase == '파일 복사' and e.total and e.current == e.total)
    after = events[copied + 1:]
    assert any('대화' in e.phase for e in after)
    verification = [e for e in after if e.phase == '백업 무결성 검증']
    assert any(e.total and 0 < e.current < e.total for e in verification)
    assert verification[-1].current == verification[-1].total
    assert verify_bundle(destination).ok
    assert (project.roots[0] / 'large.bin').is_file()


def test_backup_batch_reports_each_project_and_complete_bundles(tmp_path):
    adapter = CodexAdapter(tmp_path / 'home', isolated=True)
    projects = [make_project(adapter, tmp_path / name, name)[0] for name in ('A', 'B')]
    plans = [(p, tmp_path / 'backups' / p.name) for p in projects]
    events = []
    result = run_backups(adapter, plans, Journal(tmp_path / 'journal.sqlite'), export_project,
                         ProgressReporter(events.append, interval=0), lambda: False)
    assert result.state == 'completed'
    assert {(e.item_index, e.item_total, e.item_name) for e in events} == {(1, 2, 'A'), (2, 2, 'B')}
    assert all(verify_bundle(path).ok for _, path in plans)
