from PySide6.QtCore import Qt
from PySide6.QtWidgets import QAbstractItemView
from project_manager.models import Project,Conversation,Snapshot
from project_manager.codex.adapter import CodexAdapter
from project_manager.ui.window import MainWindow


def test_management_selection_survives_refresh_and_backup_is_readonly(qtbot,tmp_path):
    w=MainWindow(CodexAdapter(tmp_path/'home',isolated=True),tmp_path/'state',auto_refresh=False);qtbot.addWidget(w)
    p=Project('p','P',(tmp_path/'missing',))
    ts=tuple(Conversation(str(i),'p',tmp_path,(),False,None,'',False,'대화 '+str(i)) for i in range(3))
    w.refresh(Snapshot((p,),ts,''));w.project_list.setCurrentRow(0)
    w.thread_list.item(0).setSelected(True);w.thread_list.item(2).setSelected(True)
    w.filter_list()
    assert set(w.selected_thread_ids())=={'0','2'}
    assert w.thread_manage.isEnabled()
    w.set_mode('backups');assert not w.thread_manage.isEnabled()
    qtbot.waitUntil(lambda:w.size_scanner.worker is None,timeout=3000)


def test_rename_captures_identity_before_modal_refresh(qtbot,tmp_path,monkeypatch):
    w=MainWindow(CodexAdapter(tmp_path/'home',isolated=True),tmp_path/'state',auto_refresh=False);qtbot.addWidget(w)
    p=Project('p','P',(tmp_path/'missing',));t=Conversation('t','p',tmp_path,(),False,None,'',False,'원래 제목')
    w.refresh(Snapshot((p,),(t,),''));w.project_list.setCurrentRow(0);w.thread_list.item(0).setSelected(True)
    monkeypatch.setattr(w,'guard_change',lambda:True)
    def enter(*args,**kwargs):w.filter_list();return '새 제목',True
    monkeypatch.setattr('project_manager.ui.management.QInputDialog.getText',enter)
    jobs=[];monkeypatch.setattr(w,'run_job',lambda title,work,done:jobs.append(work))
    w.rename_selected_thread();assert len(jobs)==1
    # Exercise the callback after native list items have been replaced.
    captured=[]
    monkeypatch.setattr('project_manager.ui.management.rename_thread',lambda a,thread,name,j:captured.append((thread.id,name)))
    jobs[0](None);assert captured==[('t','새 제목')]
    qtbot.waitUntil(lambda:w.size_scanner.worker is None,timeout=3000)


def test_unassigned_multiple_selection_is_available_in_detail(qtbot,tmp_path):
    w=MainWindow(CodexAdapter(tmp_path/'home',isolated=True),tmp_path/'state',auto_refresh=False);qtbot.addWidget(w)
    ts=tuple(Conversation(str(i),None,tmp_path,(),False,None,'',False,'대화 '+str(i)) for i in range(3))
    w.refresh(Snapshot((),ts,''));w.set_mode('unassigned');w.project_list.setCurrentRow(0)
    w.project_list.item(1).setSelected(True)
    assert w.thread_list.count()==2
    w.thread_list.selectAll();w.filter_list()
    assert set(w.selected_thread_ids())=={'0','1'}
