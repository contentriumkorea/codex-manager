from pathlib import Path
from project_manager.models import Project,Conversation,Snapshot
from project_manager.ui.window import MainWindow
from project_manager.codex.adapter import CodexAdapter


def test_window_dark_catalog_and_selection(qtbot,tmp_path):
    w=MainWindow(CodexAdapter(tmp_path/'home',isolated=True),tmp_path/'state',auto_refresh=False)
    qtbot.addWidget(w)
    p=Project('p','테스트 A',(tmp_path/'A',))
    t=Conversation('t','p',tmp_path/'A',(),False,None,'1',False,'긴 한글 대화 제목')
    w.refresh(Snapshot((p,),(t,),'1'))
    assert w.project_list.count()==1
    w.project_list.setCurrentRow(0)
    assert w.thread_list.count()==1
    assert '#171717' in w.styleSheet()
    assert not w.cleanup_button.isEnabled()


def test_no_selection_disables_operations(qtbot,tmp_path):
    w=MainWindow(CodexAdapter(tmp_path/'h',isolated=True),tmp_path/'s',auto_refresh=False)
    qtbot.addWidget(w);w.refresh(Snapshot((),(),'0'))
    assert not w.move_button.isEnabled()
