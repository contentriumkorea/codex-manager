import threading
from PySide6.QtWidgets import QPushButton,QFrame
from PySide6.QtCore import Qt
from project_manager.models import Project,Snapshot,Conversation
from project_manager.codex.adapter import CodexAdapter
from project_manager.ui.window import MainWindow


def test_all_sizes_are_automatic_and_sorted_without_changing_selection(qtbot,tmp_path):
    small=tmp_path/'small';big=tmp_path/'big';small.mkdir();big.mkdir()
    (small/'a').write_bytes(b'abc');(big/'b').write_bytes(b'x'*4096)
    w=MainWindow(CodexAdapter(tmp_path/'h',isolated=True),tmp_path/'s',auto_refresh=False);qtbot.addWidget(w)
    w.refresh(Snapshot((Project('small','A',(small,)),Project('big','B',(big,))),(),'1'))
    w.project_list.setCurrentRow(0);selected=w.selected_project().id
    qtbot.waitUntil(lambda:len(w.sizes)==2,timeout=4000)
    qtbot.waitUntil(lambda:w.project_list.item(0).data(Qt.UserRole)=='big',timeout=4000)
    assert w.sizes=={'small':3,'big':4096} and w.selected_project().id==selected
    assert not any(b.text()=='용량 계산' for b in w.findChildren(QPushButton))
    assert w.project_list.frameShape()==QFrame.NoFrame


def test_size_scan_does_not_show_a_modal_or_block_close(qtbot,tmp_path,monkeypatch):
    import project_manager.ui.sizes as sizes
    entered=threading.Event()
    def wait(roots,cancelled):
        entered.set()
        while not cancelled():threading.Event().wait(.01)
        raise InterruptedError()
    monkeypatch.setattr(sizes,'measure_folders',wait)
    w=MainWindow(CodexAdapter(tmp_path/'h',isolated=True),tmp_path/'s',auto_refresh=False);qtbot.addWidget(w);w.show()
    w.refresh(Snapshot((Project('p','P',(tmp_path/'files',)),),(),'1'))
    qtbot.waitUntil(entered.is_set,timeout=2000)
    assert not w.workers and w.isEnabled()
    w.close();qtbot.waitUntil(lambda:not w.isVisible(),timeout=3000)


def test_changed_roots_and_file_changes_refresh_sizes(qtbot,tmp_path):
    a=tmp_path/'a';b=tmp_path/'b';a.mkdir();b.mkdir();(a/'f').write_bytes(b'1');(b/'f').write_bytes(b'123')
    w=MainWindow(CodexAdapter(tmp_path/'h',isolated=True),tmp_path/'s',auto_refresh=False);qtbot.addWidget(w)
    w.refresh(Snapshot((Project('p','P',(a,)),),(),'1'))
    qtbot.waitUntil(lambda:w.sizes.get('p')==1,timeout=3000)
    w.refresh(Snapshot((Project('p','P',(b,)),),(),'2'))
    qtbot.waitUntil(lambda:w.sizes.get('p')==3,timeout=3000)
    (b/'f').write_bytes(b'12345');w.refresh(w.snapshot)
    qtbot.waitUntil(lambda:w.sizes.get('p')==5,timeout=3000)


def test_folder_chat_is_visible_in_project_and_excluded_from_unassigned_view(qtbot,tmp_path):
    root=tmp_path/'files';root.mkdir()
    p=Project('p','P',(root,));t=Conversation('t',None,root,(),False,None,'',False,'폴더 대화')
    w=MainWindow(CodexAdapter(tmp_path/'h',isolated=True),tmp_path/'s',auto_refresh=False);qtbot.addWidget(w)
    w.refresh(Snapshot((p,),(t,),''));w.project_list.setCurrentRow(0)
    assert w.thread_list.count()==1 and '폴더 기준' in w.thread_list.item(0).text()
    assert not w.confirm_links.isHidden() and w.snapshot.conversations[0].project_id is None
    w.set_mode('unassigned');assert w.project_list.count()==0
    qtbot.waitUntil(lambda:w.size_scanner.worker is None,timeout=3000)
