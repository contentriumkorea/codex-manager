from PySide6.QtCore import Qt
from project_manager.models import Project,Snapshot,Conversation
from project_manager.codex.adapter import CodexAdapter
from project_manager.ui.window import MainWindow


def test_open_project_browse_back_and_switch_without_stale_paths(qtbot,tmp_path):
    a=tmp_path/'A';b=tmp_path/'B';a.mkdir();b.mkdir();child=a/'하위 폴더';child.mkdir()
    w=MainWindow(CodexAdapter(tmp_path/'home',isolated=True),tmp_path/'state',auto_refresh=False);qtbot.addWidget(w)
    w.refresh(Snapshot((Project('a','A',(a,)),Project('b','B',(b,))),(),''))
    w.project_list.setCurrentRow(0);w.open_project()
    assert w.file_browser.current_path==a
    w.file_browser.navigate(child);assert w.file_browser.current_path==child
    w.file_browser.go_back();assert w.file_browser.current_path==a
    w.show_projects();w.project_list.setCurrentRow(1);w.open_project()
    assert w.file_browser.current_path==b and len(w.file_browser.history)==1
    assert w.project_list.headerItem().text(0)=='이름'
    qtbot.waitUntil(lambda:w.size_scanner.worker is None,timeout=3000)


def test_missing_folder_and_outside_path_do_not_open_another_project(qtbot,tmp_path):
    from project_manager.ui.explorer import FileBrowser
    a=tmp_path/'A';a.mkdir();outside=tmp_path/'B';outside.mkdir()
    browser=FileBrowser();qtbot.addWidget(browser);browser.set_project(Project('a','A',(a,)))
    assert not browser.navigate(outside) and browser.current_path==a
    browser.set_project(Project('missing','Missing',(tmp_path/'missing',)))
    assert browser.current_path is None and '찾을 수' in browser.status.text()


def test_search_finds_folder_path_and_clears_hidden_file_context(qtbot,tmp_path):
    a=tmp_path/'special-folder';a.mkdir();b=tmp_path/'B';b.mkdir()
    w=MainWindow(CodexAdapter(tmp_path/'h',isolated=True),tmp_path/'s',auto_refresh=False);qtbot.addWidget(w)
    w.refresh(Snapshot((Project('a','Alpha',(a,)),Project('b','Beta',(b,))),(),''))
    w.project_list.setCurrentRow(0);w.open_project();w.search.setText('special-folder')
    assert w.project_list.count()==1 and w.selected_project().id=='a'
    w.search.setText('no-such-folder')
    assert w.selected_project() is None and w.browser_stack.currentWidget()!=w.file_browser
    assert not w.move_button.isEnabled()
    qtbot.waitUntil(lambda:w.size_scanner.worker is None,timeout=3000)


def test_transcript_survives_catalog_refresh_while_reading(qtbot,tmp_path,monkeypatch):
    w=MainWindow(CodexAdapter(tmp_path/'h',isolated=True),tmp_path/'s',auto_refresh=False);qtbot.addWidget(w)
    t=Conversation('t',None,tmp_path,(),False,None,'',False,'대화 제목',tmp_path/'rollout.jsonl')
    w.refresh(Snapshot((),(t,),''));w.set_mode('unassigned');w.project_list.setCurrentRow(0)
    pending=[];titles=[]
    monkeypatch.setattr(w,'run_job',lambda title,work,done:pending.append((work,done)))
    monkeypatch.setattr('project_manager.ui.window.transcript',lambda path:[{'role':'user','text':'안녕'}])
    monkeypatch.setattr('project_manager.ui.window.QDialog.exec',lambda dialog:titles.append(dialog.windowTitle()))
    w.show_transcript(w.project_list.currentItem());w.filter_list()
    work,done=pending[0];done(work(None))
    assert titles==['대화 제목'] and w.project_list.currentItem().data(Qt.UserRole)=='t'


def test_assign_picker_survives_background_refresh(qtbot,tmp_path,monkeypatch):
    w=MainWindow(CodexAdapter(tmp_path/'h',isolated=True),tmp_path/'s',auto_refresh=False);qtbot.addWidget(w)
    p=Project('p','P',(tmp_path/'files',));t=Conversation('t',None,tmp_path/'outside',(),False,None,'',False,'대화')
    w.refresh(Snapshot((p,),(t,),''));w.set_mode('unassigned');w.project_list.setCurrentRow(0)
    monkeypatch.setattr(w,'guard_change',lambda:True);assignments=[]
    def choose(*args):w.filter_list();return 'P · p',True
    monkeypatch.setattr('project_manager.ui.window.QInputDialog.getItem',choose)
    monkeypatch.setattr('project_manager.ui.window.change_connections',lambda adapter,project,values,journal:assignments.append(values))
    monkeypatch.setattr(w,'run_job',lambda title,work,done:work(None))
    w.assign_unassigned();assert assignments==[{'t':'p'}]
    qtbot.waitUntil(lambda:w.size_scanner.worker is None,timeout=3000)


def test_backup_finishes_in_selected_backup_with_restore_available(qtbot,tmp_path,monkeypatch):
    from types import SimpleNamespace
    root=tmp_path/'files';root.mkdir();(root/'note.txt').write_text('keep original')
    backups=tmp_path/'backups';backups.mkdir()
    adapter=CodexAdapter(tmp_path/'h',isolated=True)
    snapshot=Snapshot((Project('p','P',(root,)),),(),'')
    monkeypatch.setattr(adapter,'snapshot',lambda *a,**kw:snapshot)
    w=MainWindow(adapter,tmp_path/'s',auto_refresh=False);qtbot.addWidget(w);w.refresh(snapshot);w.project_list.setCurrentRow(0)
    monkeypatch.setattr('project_manager.ui.window.QFileDialog.getExistingDirectory',lambda *args:str(backups))
    monkeypatch.setattr('project_manager.ui.window.confirm',lambda *args:True)
    monkeypatch.setattr('project_manager.ui.window.QMessageBox.information',lambda *args:None)
    worker=SimpleNamespace(progress=SimpleNamespace(emit=lambda *args:None),cancelled=SimpleNamespace(is_set=lambda:False))
    monkeypatch.setattr(w,'run_job',lambda title,work,done:done(work(worker)))
    w.backup()
    assert w.mode=='backups' and w.active_bundle==w.backups[-1] and w.bundle_action.isEnabled()
    assert (root/'note.txt').read_text()=='keep original'
    qtbot.waitUntil(lambda:w.size_scanner.worker is None,timeout=3000)


def test_unassigned_internal_records_are_optional_and_retained(qtbot,tmp_path):
    from dataclasses import replace
    w=MainWindow(CodexAdapter(tmp_path/'h',isolated=True),tmp_path/'s',auto_refresh=False);qtbot.addWidget(w)
    t=Conversation('user',None,tmp_path,(),False,None,'',False,'User')
    internal=replace(t,id='internal',title='Guardian review',internal=True)
    w.refresh(Snapshot((),(t,internal),''));w.set_mode('unassigned')
    assert w.project_list.count()==1
    w.show_internal.setChecked(True)
    assert w.project_list.count()==2 and len(w.snapshot.conversations)==2
    w.show_internal.setChecked(False)
    assert w.project_list.count()==1
