from project_manager.codex.adapter import CodexAdapter
from project_manager.journal import Journal
from project_manager.models import OperationResult
from project_manager.ui.batch import plan_projects, run_projects
from test_operations import make_project


def test_batch_report_persists_partial_results_and_safe_retry_candidates(tmp_path, monkeypatch):
    a = CodexAdapter(tmp_path / 'home', isolated=True)
    projects = tuple(make_project(a, tmp_path / name, name)[0] for name in ('A', 'B', 'C'))
    journal = Journal(tmp_path / 'journal.sqlite')
    calls = []
    def transfer(adapter, project, *args):
        calls.append(project.id)
        if len(calls) == 2:
            raise ValueError('destination unavailable')
        return OperationResult('completed', 'copied', 'linked', 'kept')
    monkeypatch.setattr('project_manager.ui.batch.transfer_with_connections', transfer)
    result = run_projects(a, plan_projects(projects, tmp_path / 'out'), None, False, journal, {}, lambda *a: None, lambda: False)
    report = journal.operation(result.report_id)
    items = report['payload']['items']
    assert [i['status'] for i in items] == ['completed', 'failed', 'not_started']
    assert items[1]['errors'] == ['destination unavailable']
    assert items[0]['destinations'] == [str(tmp_path / 'out' / 'A')]
    from project_manager.ui.activity import retry_candidates
    assert retry_candidates(report, journal) == (projects[1].id, projects[2].id)
    journal.begin('pending', {'kind': 'move', 'home': str(a.home), 'resources': [projects[1].id]})
    assert retry_candidates(report, journal) == ()
    journal.record('pending', 'completed', {'recovered': True})
    assert retry_candidates(report, journal) == (projects[1].id, projects[2].id)
    journal.record(result.report_id, 'completed', {'retry_report': 'new-report'})
    assert retry_candidates(journal.operation(result.report_id), journal) == ()


def test_history_filters_home_before_limit_and_never_promises_undo_for_completed_move(tmp_path):
    journal = Journal(tmp_path / 'journal.sqlite')
    home = tmp_path / 'home'
    for key, kind, location in [('move', 'move', home), ('rename', 'rename-thread', home), ('foreign', 'file-edit', tmp_path / 'other')]:
        journal.begin(key, {'kind': kind, 'home': str(location), 'resources': []})
        journal.record(key, 'completed', {})
    records = journal.history(home, limit=2)
    assert [r['id'] for r in records] == ['rename', 'move']
    from project_manager.ui.activity import recovery_available
    assert recovery_available(records[0], journal)
    assert not recovery_available(records[1], journal)
    journal.record('rename', 'completed', {'recovered': True})
    assert not recovery_available(journal.operation('rename'), journal)


def test_cancelled_batch_has_no_failure_or_completed_items(tmp_path):
    a = CodexAdapter(tmp_path / 'home', isolated=True)
    p, _ = make_project(a, tmp_path / 'A', 'A')
    journal = Journal(tmp_path / 'journal.sqlite')
    result = run_projects(a, plan_projects((p,), tmp_path / 'out'), None, False, journal, {}, lambda *a: None, lambda: True)
    report = journal.operation(result.report_id)
    assert report['payload']['items'][0]['status'] == 'not_started'
    assert report['payload']['result_state'] == 'cancelled'
    assert not journal.pending()


def test_backup_plan_and_completed_items_survive_process_exit(tmp_path):
    import pytest
    from project_manager.ui.batch import run_backups
    from project_manager.models import Verification
    a = CodexAdapter(tmp_path / 'home', isolated=True)
    ps = tuple(make_project(a, tmp_path / name, name)[0] for name in ('A', 'B', 'C'))
    journal = Journal(tmp_path / 'journal.sqlite'); calls = []
    plans = [(p, tmp_path / 'backup' / p.name) for p in ps]
    def export(p, *args, **kwargs):
        calls.append(p)
        if len(calls) == 2: raise SystemExit('process stopped')
        return Verification(True)
    with pytest.raises(SystemExit):
        run_backups(a, plans, journal, export, lambda *a: None, lambda: False)
    reopened = Journal(journal.path); report = reopened.history(a.home)[0]
    assert [i['status'] for i in report['payload']['items']] == ['completed', 'failed', 'not_started']
    assert reopened.verified_backups(a.home) == [plans[0][1]]
    from project_manager.ui.activity import retry_candidates
    assert retry_candidates(report, reopened) == (ps[1].id, ps[2].id)


def test_transfer_commit_before_report_save_cannot_be_retried_as_unfinished(tmp_path, monkeypatch):
    import pytest
    a = CodexAdapter(tmp_path / 'home', isolated=True)
    ps = tuple(make_project(a, tmp_path / name, name)[0] for name in ('A', 'B'))
    journal = Journal(tmp_path / 'journal.sqlite'); update = journal.update_report
    def stop_after_child(key, payload):
        if payload['items'][0]['status'] == 'completed': raise SystemExit('crash gap')
        update(key, payload)
    monkeypatch.setattr(journal, 'update_report', stop_after_child)
    with pytest.raises(SystemExit):
        run_projects(a, plan_projects(ps, tmp_path / 'out'), None, False, journal, {}, lambda *a: None, lambda: False)
    reopened = Journal(journal.path)
    report = next(r for r in reopened.history(a.home) if r['payload']['kind'] == 'batch-summary')
    assert [i['status'] for i in report['payload']['items']] == ['completed', 'not_started']
    from project_manager.ui.activity import retry_candidates
    assert retry_candidates(report, reopened) == (ps[1].id,)


def test_verified_backup_commit_gap_is_reconciled_from_export_proof(tmp_path, monkeypatch):
    import pytest
    from project_manager.bundles import export_project, verify_bundle
    from project_manager.ui.batch import run_backups
    a = CodexAdapter(tmp_path / 'home', isolated=True)
    p, _ = make_project(a, tmp_path / 'A', 'A'); destination = tmp_path / 'backup'
    journal = Journal(tmp_path / 'journal.sqlite'); update = journal.update_report
    def stop_before_report(key, payload):
        if payload['items'][0]['status'] == 'completed': raise SystemExit('crash gap')
        update(key, payload)
    monkeypatch.setattr(journal, 'update_report', stop_before_report)
    with pytest.raises(SystemExit):
        run_backups(a, [(p, destination)], journal, export_project, lambda *a: None, lambda: False)
    assert verify_bundle(destination).ok
    reopened = Journal(journal.path); report = reopened.history(a.home)[0]
    assert report['payload']['items'][0]['status'] == 'completed'
    assert reopened.verified_backups(a.home) == [destination]
    from project_manager.ui.activity import retry_candidates
    assert not retry_candidates(report, reopened)
    # A recorded historical verification survives an unplugged drive.
    import json
    proof = destination / 'verification.json'; proof.unlink()
    assert reopened.verified_backups(a.home) == [destination]
    assert not retry_candidates(reopened.operation(report['id']), reopened)
    # A fresh incomplete bundle without recorded evidence is never registered.
    other = Journal(tmp_path / 'other-journal.sqlite')
    other_id = other.report({'home': str(a.home), 'action': 'backup', 'result_state': 'running', 'items': [dict(report['payload']['items'][0], status='in_progress')]})
    proof.write_text(json.dumps({'integrity': False}))
    assert not other.verified_backups(a.home)


def test_retry_relation_survives_exit_before_result_callback(tmp_path):
    import pytest
    from project_manager.ui.batch import run_backups
    from project_manager.models import Verification
    from project_manager.ui.activity import retry_candidates
    a = CodexAdapter(tmp_path / 'home', isolated=True)
    ps = tuple(make_project(a, tmp_path / name, name)[0] for name in ('A', 'B'))
    journal = Journal(tmp_path / 'journal.sqlite'); calls = []
    old = run_backups(a, [(p, tmp_path / 'old' / p.name) for p in ps], journal, lambda *a, **k: Verification(False, ('offline',)), lambda *a: None, lambda: False)
    def export(project, *args, **kwargs):
        calls.append(project)
        if len(calls) == 2: raise SystemExit('exit during retry')
        return Verification(True)
    with pytest.raises(SystemExit):
        run_backups(a, [(p, tmp_path / 'new' / p.name) for p in ps], journal, export, lambda *a: None, lambda: False, retry_of=old.report_id)
    reopened = Journal(journal.path)
    assert not retry_candidates(reopened.operation(old.report_id), reopened)
    latest = reopened.history(a.home)[0]
    assert retry_candidates(latest, reopened) == (ps[1].id,)
    with pytest.raises(ValueError, match='이미 재시도'):
        reopened.report({'home': str(a.home), 'retry_of': old.report_id})
