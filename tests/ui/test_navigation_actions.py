from PySide6.QtCore import Qt
from project_manager.codex.adapter import CodexAdapter
from project_manager.models import Project, Conversation, Snapshot
from project_manager.ui.window import MainWindow


def window(qtbot, tmp_path):
    roots = tuple(tmp_path / name for name in ('Alpha', 'Beta'))
    for root in roots:
        root.mkdir()
    (roots[0] / 'one.txt').write_text('one')
    w = MainWindow(CodexAdapter(tmp_path / 'home', isolated=True), tmp_path / 'state', auto_refresh=False)
    qtbot.addWidget(w)
    ps = tuple(Project(str(i), root.name, (root,)) for i, root in enumerate(roots))
    ts = tuple(Conversation('t' + str(i), '0', roots[0], (), False, None, '', False, 'Chat ' + str(i)) for i in range(2))
    w.refresh(Snapshot(ps, ts, '')); w.show(); w.project_list.setCurrentRow(0)
    qtbot.waitUntil(lambda: w.size_scanner.worker is None, timeout=3000)
    return w, roots


def test_navigation_survives_file_mode_and_clears_hidden_search(qtbot, tmp_path):
    w, roots = window(qtbot, tmp_path)
    w.open_project()
    assert w.navigator.isVisible()
    w.search.setText('Alpha')
    w.navigator.activate_project('1', roots[1])
    assert w.selected_project().id == '1'
    assert w.file_browser.current_path == roots[1]
    assert w.browser_stack.currentWidget() == w.file_browser
    assert w.search.text() == ''


def test_favorites_are_persistent_and_scoped_to_store(qtbot, tmp_path):
    w, _ = window(qtbot, tmp_path)
    w.navigator.toggle_favorite('0')
    from project_manager.settings import read_settings
    assert '0' in read_settings(w.state_dir / 'settings.json')['favorite_projects'][str(w.adapter.home)]
    original = w.adapter
    w.adapter = CodexAdapter(tmp_path / 'other-home', isolated=True)
    w.refresh(w.snapshot)
    assert not w.navigator.favorites()
    w.adapter = original; w.refresh(w.snapshot)
    assert w.navigator.favorites() == {'0'}


def test_sidebar_drop_keeps_chat_selection_and_targets_visible_folder(qtbot, tmp_path):
    w, roots = window(qtbot, tmp_path)
    w.thread_list.selectAll()
    mime = w.dragdrop.mime_for(w.thread_list)
    tree = w.navigator.tree
    target = w.navigator.project_item('1')
    plan = w.dragdrop.plan(mime, tree, tree.visualItemRect(target).center(), Qt.NoModifier)
    assert plan['target'].id == '1' and len(plan['threads']) == 2
    assert w.selected_project().id == '0'
    w.open_project()
    assert w.dragdrop.plan(mime, tree, tree.visualItemRect(target).center(), Qt.NoModifier)['target'].id == '1'


def test_action_bar_tracks_active_list_not_hidden_selection(qtbot, tmp_path, monkeypatch):
    w, roots = window(qtbot, tmp_path)
    w.project_list.selectAll(); w.selection_bar.activate(w.project_list)
    assert w.selection_bar.summary.text() == '프로젝트 2개 선택'
    assert w.selection_bar.buttons['backup'].isVisible()
    w.project_list.setCurrentRow(0); w.open_project()
    b = w.file_browser
    qtbot.waitUntil(lambda: b.model.index(str(roots[0] / 'one.txt')).isValid())
    b.view.setCurrentIndex(b.model.index(str(roots[0] / 'one.txt')))
    w.selection_bar.activate(b.view)
    assert w.selection_bar.summary.text() == '파일·폴더 1개 선택'
    w.thread_list.selectAll(); w.selection_bar.activate(w.thread_list)
    assert w.selection_bar.summary.text() == '대화 2개 선택'
    calls = []; monkeypatch.setattr(w, 'move_selected_threads', lambda ids: calls.append(ids))
    w.selection_bar.buttons['move'].click()
    assert set(calls[0]) == {'t0', 't1'}
    w.set_mode('backups')
    assert not w.selection_bar.isVisible()


def test_toolbar_does_not_run_while_busy(qtbot, tmp_path, monkeypatch):
    w, _ = window(qtbot, tmp_path)
    w.project_list.selectAll(); w.selection_bar.activate(w.project_list)
    calls = []; monkeypatch.setattr(w, 'move', lambda: calls.append(True))
    w.updater.requested = True
    w.selection_bar.dispatch('move')
    assert not calls
    w.updater.requested = False


def test_changing_file_folder_clears_hidden_selection(qtbot, tmp_path):
    w, roots = window(qtbot, tmp_path); w.open_project(); b = w.file_browser
    folder = roots[0] / 'sub'; folder.mkdir()
    qtbot.waitUntil(lambda: b.model.index(str(roots[0] / 'one.txt')).isValid())
    b.view.setCurrentIndex(b.model.index(str(roots[0] / 'one.txt')))
    w.selection_bar.activate(b.view); assert w.selection_bar.isVisible()
    b.navigate(folder)
    assert not w.shortcuts.paths() and not w.selection_bar.isVisible()


def test_backup_partial_failure_registers_only_verified_items(qtbot, tmp_path, monkeypatch):
    from types import SimpleNamespace
    from project_manager.models import Verification
    w, _ = window(qtbot, tmp_path); w.project_list.selectAll()
    snapshot = w.snapshot; monkeypatch.setattr(w.adapter, 'snapshot', lambda *a, **kw: snapshot)
    monkeypatch.setattr('project_manager.ui.window.QFileDialog.getExistingDirectory', lambda *args: str(tmp_path / 'backup'))
    monkeypatch.setattr('project_manager.ui.window.confirm', lambda *args: True)
    calls = []
    def export(project, snapshot, destination, *_args, **_kwargs):
        calls.append(destination)
        return Verification(len(calls) == 1, () if len(calls) == 1 else ('disk unavailable',))
    monkeypatch.setattr('project_manager.ui.window.export_project', export)
    worker = SimpleNamespace(progress=SimpleNamespace(emit=lambda *args: None), cancelled=SimpleNamespace(is_set=lambda: False))
    monkeypatch.setattr(w, 'run_job', lambda title, work, done: done(work(worker)))
    w.backup()
    assert w.backups == [calls[0]]
    assert w.activity_dialog.current['payload']['items'][1]['status'] == 'failed'
    assert 'disk unavailable' in w.activity_dialog.detail.toPlainText()
    assert w.activity_dialog.retry.isEnabled() and not w.activity_dialog.recover.isEnabled()


def test_history_recovery_routes_exact_operation_without_changing_project_selection(qtbot, tmp_path, monkeypatch):
    w, _ = window(qtbot, tmp_path)
    w.journal.begin('rename', {'kind': 'rename-thread', 'home': str(w.adapter.home), 'resources': [], 'old_name': 'Old', 'new_name': 'New'})
    w.journal.record('rename', 'completed', {})
    dialog = w.open_activity()
    assert dialog.recover.isEnabled() and not dialog.retry.isEnabled()
    calls = []; monkeypatch.setattr(w, 'show_recovery', lambda operation_id: calls.append(operation_id))
    dialog.recover.click()
    assert calls == ['rename'] and w.selected_project().id == '0'


def test_sidebar_folder_drop_uses_that_root_and_readonly_modes_reject(qtbot, tmp_path):
    from PySide6.QtCore import QMimeData, QUrl
    w, roots = window(qtbot, tmp_path)
    tree = w.navigator.tree; target = w.navigator.project_item('1'); target.setExpanded(True)
    mime = QMimeData(); mime.setUrls([QUrl.fromLocalFile(str(roots[0] / 'one.txt'))])
    plan = w.dragdrop.plan(mime, tree, tree.visualItemRect(target.child(0)).center(), Qt.NoModifier)
    assert plan['destination'] == roots[1] and plan['action'] == 'copy'
    w.set_mode('backups')
    assert w.dragdrop.plan(mime, tree, tree.visualItemRect(target).center(), Qt.NoModifier) is None


def test_hidden_chat_tab_does_not_keep_destructive_actions(qtbot, tmp_path):
    w, _ = window(qtbot, tmp_path)
    w.thread_list.selectAll(); w.selection_bar.activate(w.thread_list)
    assert w.selection_bar.isVisible()
    w.detail_tabs.setCurrentWidget(w.folder_list)
    assert not w.selection_bar.isVisible() and w.selection_bar.selection() == (None, ())


def test_single_result_selects_its_operation_and_noop_clears_old_recovery(qtbot, tmp_path):
    from project_manager.file_edits import edit_files
    from project_manager.models import OperationResult
    w, roots = window(qtbot, tmp_path)
    first = edit_files('copy', (roots[0] / 'one.txt',), roots[1], roots, (), w.journal, w.adapter.home)
    d = w.open_activity(); d.show_result(first)
    second = edit_files('delete', (roots[1] / 'one.txt',), roots[1], roots, (), w.journal, w.adapter.home)
    d.show_result(second)
    assert second.report_id and d.current['id'] == second.report_id
    d.show_result(OperationResult('completed', '변경 없음', '대화 유지', '파일 유지'))
    assert d.current is None and not d.recover.isEnabled()
