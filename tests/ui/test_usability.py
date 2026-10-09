from dataclasses import replace
from PySide6.QtCore import Qt
from PySide6.QtWidgets import QDialogButtonBox
from project_manager.models import Project,Conversation,Snapshot
from project_manager.codex.adapter import CodexAdapter
from project_manager.ui.window import MainWindow
from project_manager.ui.naming import RenameDialog


def test_rename_preview_requires_valid_actual_change(qtbot):
    d=RenameDialog(None,['Alpha']);qtbot.addWidget(d)
    ok=d.buttons.button(QDialogButtonBox.Ok)
    assert not ok.isEnabled()
    d.name.setText('  Beta  ')
    assert ok.isEnabled() and d.names()==('Beta',)
    d.name.setText('   ');assert not ok.isEnabled()


def test_bulk_preview_lists_exact_changes(qtbot):
    d=RenameDialog(None,['Alpha','Beta']);qtbot.addWidget(d)
    d.prefix.setText('2026 ')
    assert d.names()==('2026 Alpha','2026 Beta')
    assert d.preview.topLevelItem(1).text(1)=='2026 Beta'


def test_all_conversations_filter_search_selection(qtbot,tmp_path):
    w=MainWindow(CodexAdapter(tmp_path/'h',isolated=True),tmp_path/'s',auto_refresh=False);qtbot.addWidget(w)
    t=Conversation('a','p',tmp_path,(),False,None,'',False,'Alpha',updated_at=10)
    ts=(t,replace(t,id='b',title='Beta',archived=True,updated_at=20),replace(t,id='c',internal=True,title='Internal'))
    w.refresh(Snapshot((Project('p','Project',(tmp_path/'files',)),),ts,''));w.set_mode('conversations')
    assert w.project_list.count()==2
    w.project_list.selectAll();assert set(w.selected_thread_ids())=={'a','b'}
    assert w.thread_list.count()==2 and w.assign_action.isEnabled()
    from PySide6.QtWidgets import QApplication
    w.copy_thread_titles(('b',));assert QApplication.clipboard().text()=='Beta'
    w.chat_status.setCurrentIndex(1);assert w.project_list.count()==1 and w.selected_thread_ids()==('a',)
    w.search.setText('no match');assert not w.selected_thread_ids()
    w.search.clear();w.chat_status.setCurrentIndex(0);w.project_list.setCurrentRow(0)
    chosen=w.selected_thread_ids();w.filter_list();assert w.selected_thread_ids()==chosen
    w.set_mode('backups');assert not w.selected_thread_ids()
    qtbot.waitUntil(lambda:w.size_scanner.worker is None,timeout=3000)


def test_destination_search_keeps_project_identity(qtbot,tmp_path):
    from project_manager.ui.naming import ProjectPicker
    ps=(Project('a','Same',(tmp_path/'a',)),Project('b','Same',(tmp_path/'b',)))
    d=ProjectPicker(None,ps,'Move','Pick target');qtbot.addWidget(d)
    d.search.setText(str(tmp_path/'b'))
    assert d.list.count()==1
    d.list.setCurrentRow(0);assert d.project_id()=='b'
    d.search.setText('missing');assert not d.buttons.button(QDialogButtonBox.Ok).isEnabled()


def test_search_remembered_per_section(qtbot,tmp_path):
    w=MainWindow(CodexAdapter(tmp_path/'h',isolated=True),tmp_path/'s',auto_refresh=False);qtbot.addWidget(w)
    w.search.setText('project query');w.set_mode('conversations');assert w.search.text()==''
    w.search.setText('chat query');w.set_mode('projects');assert w.search.text()=='project query'


def test_backup_shortcuts_cannot_target_live_threads(qtbot,tmp_path,monkeypatch):
    w=MainWindow(CodexAdapter(tmp_path/'h',isolated=True),tmp_path/'s',auto_refresh=False);qtbot.addWidget(w)
    t=Conversation('same',None,tmp_path,(),False,None,'',False,'Live')
    w.refresh(Snapshot((),(t,),''));w.set_mode('backups')
    assert w.selected_threads(('same',))==()
    monkeypatch.setattr(w,'guard_change',lambda:(_ for _ in ()).throw(AssertionError('backup mutation')))
    w.rename_selected_thread(('same',));w.move_selected_threads(('same',));w.delete_selected_threads(('same',))
