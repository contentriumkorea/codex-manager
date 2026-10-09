from PySide6.QtCore import Qt
from PySide6.QtWidgets import QApplication
from project_manager.models import Project,Conversation,Snapshot
from project_manager.codex.adapter import CodexAdapter
from project_manager.ui.window import MainWindow


def window(qtbot,tmp_path):
    root=tmp_path/'files';root.mkdir();(root/'a.txt').write_text('a')
    w=MainWindow(CodexAdapter(tmp_path/'home',isolated=True),tmp_path/'state',auto_refresh=False);qtbot.addWidget(w)
    t=Conversation('t','p',root,(),False,None,'',False,'Example')
    w.refresh(Snapshot((Project('p','P',(root,)),),(t,),''));w.project_list.setCurrentRow(0)
    return w,root


def test_text_edits_keep_native_clipboard(qtbot,tmp_path):
    w,root=window(qtbot,tmp_path);w.show();w.search.setText('hello');w.search.setFocus();w.search.selectAll()
    qtbot.keyClick(w.search,Qt.Key_C,Qt.ControlModifier)
    assert QApplication.clipboard().text()=='hello'
    qtbot.keyClick(w.search,Qt.Key_X,Qt.ControlModifier);assert w.search.text()==''
    qtbot.keyClick(w.search,Qt.Key_V,Qt.ControlModifier);assert w.search.text()=='hello'
    qtbot.waitUntil(lambda:w.size_scanner.worker is None,timeout=3000)


def test_chat_cut_is_non_destructive_and_clipboard_home_bound(qtbot,tmp_path):
    w,root=window(qtbot,tmp_path);w.set_mode('conversations');w.project_list.setCurrentRow(0)
    w.shortcuts.copy(w.project_list,True)
    assert w.shortcuts.cut['kind']=='threads' and w.shortcuts.cut['ids']==('t',)
    assert w.snapshot.conversations[0].project_id=='p'
    QApplication.clipboard().setText('different')
    qtbot.waitUntil(lambda:w.shortcuts.cut is None,timeout=3000)
    qtbot.waitUntil(lambda:w.size_scanner.worker is None,timeout=3000)


def test_file_clipboard_urls_and_backup_cut_guard(qtbot,tmp_path):
    w,root=window(qtbot,tmp_path);w.open_project()
    qtbot.waitUntil(lambda:w.file_browser.model.index(str(root/'a.txt')).isValid(),timeout=3000)
    w.file_browser.view.setCurrentIndex(w.file_browser.model.index(str(root/'a.txt')))
    w.shortcuts.copy(w.file_browser.view,False)
    assert QApplication.clipboard().mimeData().urls()[0].toLocalFile()==str(root/'a.txt').replace('\\','/') or QApplication.clipboard().mimeData().urls()[0].toLocalFile()==str(root/'a.txt')
    w.set_mode('backups');w.shortcuts.copy(w.thread_list,True);qtbot.waitUntil(lambda:w.shortcuts.cut is None,timeout=3000)
    qtbot.waitUntil(lambda:w.size_scanner.worker is None,timeout=3000)


def test_real_key_actions_route_to_focused_list(qtbot,tmp_path,monkeypatch):
    w,root=window(qtbot,tmp_path);w.show();w.activateWindow();w.set_mode('conversations');w.project_list.setCurrentRow(0);w.project_list.setFocus();qtbot.wait(80)
    qtbot.keyClick(w.project_list,Qt.Key_X,Qt.ControlModifier)
    assert w.shortcuts.cut and w.shortcuts.cut['ids']==('t',)
    qtbot.keyClick(w.project_list,Qt.Key_Escape);assert w.shortcuts.cut is None
    events=[];monkeypatch.setattr(w.shortcuts,'delete',lambda view:events.append(('delete',view)))
    monkeypatch.setattr(w.shortcuts,'paste',lambda view:events.append(('paste',view)))
    qtbot.keyClick(w.project_list,Qt.Key_Delete);qtbot.keyClick(w.project_list,Qt.Key_V,Qt.ControlModifier)
    assert events==[('delete',w.project_list),('paste',w.project_list)]
    qtbot.waitUntil(lambda:w.size_scanner.worker is None,timeout=3000)
