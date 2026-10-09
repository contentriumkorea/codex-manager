from PySide6.QtCore import Qt, QMimeData, QUrl, QPointF
from PySide6.QtGui import QDragEnterEvent,QDropEvent
from PySide6.QtWidgets import QApplication,QAbstractItemView
from project_manager.models import Project,Conversation,Snapshot
from project_manager.codex.adapter import CodexAdapter
from project_manager.ui.window import MainWindow


def setup(qtbot,tmp_path):
    roots=[tmp_path/name for name in ('A','B','C')]
    for r in roots:r.mkdir()
    (roots[0]/'file.txt').write_text('original');(roots[0]/'folder').mkdir()
    w=MainWindow(CodexAdapter(tmp_path/'home',isolated=True),tmp_path/'state',auto_refresh=False);qtbot.addWidget(w)
    projects=tuple(Project(str(i),r.name,(r,)) for i,r in enumerate(roots))
    chats=tuple(Conversation('t'+str(i),'0',roots[0],(),False,None,'',False,'Chat '+str(i)) for i in range(2))
    w.refresh(Snapshot(projects,chats,''));w.show();w.project_list.setCurrentRow(0)
    qtbot.waitUntil(lambda:w.size_scanner.worker is None,timeout=3000)
    return w,roots


def point(w,key):
    item=next(w.project_list.item(i) for i in range(w.project_list.count()) if w.project_list.item(i).data(Qt.UserRole)==key)
    return w.project_list.visualItemRect(item).center()


def test_multiple_projects_drag_plan_and_self_rejection(qtbot,tmp_path):
    w,roots=setup(qtbot,tmp_path)
    assert w.project_list.selectionMode()==QAbstractItemView.ExtendedSelection
    w.project_list.item(0).setSelected(True);w.project_list.item(1).setSelected(True)
    mime=w.dragdrop.mime_for(w.project_list)
    plan=w.dragdrop.plan(mime,w.project_list,point(w,'2'),Qt.NoModifier)
    assert plan['kind']=='projects' and len(plan['sources'])==2 and plan['target'].id=='2'
    assert w.dragdrop.plan(mime,w.project_list,point(w,'0'),Qt.NoModifier) is None
    assert w.selected_project_ids()==('0','1')
    w.filter_list();assert set(w.selected_project_ids())=={'0','1'}


def test_chat_drag_payload_survives_selection_change_and_blocks_other_home(qtbot,tmp_path):
    w,roots=setup(qtbot,tmp_path);w.thread_list.selectAll()
    mime=w.dragdrop.mime_for(w.thread_list)
    w.project_list.setCurrentRow(1)
    plan=w.dragdrop.plan(mime,w.project_list,point(w,'1'),Qt.NoModifier)
    assert {t.id for t in plan['threads']}=={'t0','t1'}
    w.adapter=CodexAdapter(tmp_path/'other',isolated=True)
    assert w.dragdrop.plan(mime,w.project_list,point(w,'1'),Qt.NoModifier) is None


def test_external_files_copy_only_and_real_drop_routing(qtbot,tmp_path,monkeypatch):
    w,roots=setup(qtbot,tmp_path);mime=QMimeData();mime.setUrls([QUrl.fromLocalFile(str(roots[0]/'file.txt'))])
    p=point(w,'1');plan=w.dragdrop.plan(mime,w.project_list,p,Qt.NoModifier)
    assert plan['kind']=='files' and plan['action']=='copy' and plan['destination']==roots[1]
    calls=[];monkeypatch.setattr(w.dragdrop,'perform',lambda plan:calls.append(plan))
    enter=QDragEnterEvent(p,Qt.CopyAction|Qt.MoveAction,mime,Qt.LeftButton,Qt.NoModifier)
    QApplication.sendEvent(w.project_list.viewport(),enter);assert enter.isAccepted()
    drop=QDropEvent(QPointF(p),Qt.CopyAction|Qt.MoveAction,mime,Qt.LeftButton,Qt.NoModifier)
    QApplication.sendEvent(w.project_list.viewport(),drop)
    qtbot.waitUntil(lambda:bool(calls));assert drop.isAccepted() and drop.dropAction()==Qt.CopyAction
    assert calls[0]['destination']==roots[1]
    assert (roots[0]/'file.txt').exists()
    w.set_mode('backups');assert w.dragdrop.plan(mime,w.project_list,p,Qt.NoModifier) is None


def test_internal_file_move_ctrl_copy_and_descendant_rejection(qtbot,tmp_path):
    w,roots=setup(qtbot,tmp_path);w.open_project();b=w.file_browser
    qtbot.waitUntil(lambda:b.model.index(str(roots[0]/'file.txt')).isValid())
    b.view.setCurrentIndex(b.model.index(str(roots[0]/'file.txt')))
    mime=w.dragdrop.mime_for(b.view);idx=b.model.index(str(roots[0]/'folder'));pos=b.view.visualRect(idx).center()
    assert w.dragdrop.plan(mime,b.view,pos,Qt.NoModifier)['action']=='move'
    assert w.dragdrop.plan(mime,b.view,pos,Qt.ControlModifier)['action']=='copy'
    b.view.setCurrentIndex(idx);mime=w.dragdrop.mime_for(b.view)
    assert w.dragdrop.plan(mime,b.view,pos,Qt.NoModifier) is None


def test_header_sort_toggle_numeric_selection_and_persistence(qtbot,tmp_path):
    w,roots=setup(qtbot,tmp_path)
    w.project_list.item(0).setSelected(True);w.project_list.item(1).setSelected(True)
    w.column_sort.clicked(0)
    assert [w.project_list.item(i).text() for i in range(3)]==['A','B','C']
    w.column_sort.clicked(0)
    assert [w.project_list.item(i).text() for i in range(3)]==['C','B','A']
    assert set(w.selected_project_ids())=={'0','1'}
    w.column_sort.clicked(2)
    assert w.project_list.item(2).text()=='A'
    w.column_sort.clicked(2)
    assert w.project_list.item(0).text()=='A'
    from project_manager.settings import read_settings
    assert read_settings(w.state_dir/'settings.json')['column_sort']['projects']==[2,True]
    width=370;w.project_list.setColumnWidth(0,width);w.filter_list()
    assert w.project_list.columnWidth(0)==width


def test_mouse_gesture_starts_drag_with_multiple_selection(qtbot,tmp_path,monkeypatch):
    w,roots=setup(qtbot,tmp_path);v=w.project_list;events=[]
    v.item(0).setSelected(True);v.item(1).setSelected(True)
    monkeypatch.setattr(w.dragdrop,'begin',lambda view:events.append(w.selected_project_ids()))
    start=point(w,'0');end=point(w,'2')
    qtbot.mousePress(v.viewport(),Qt.LeftButton,pos=start)
    from PySide6.QtGui import QMouseEvent
    from PySide6.QtCore import QEvent,QPointF
    event=QMouseEvent(QEvent.MouseMove,QPointF(end),QPointF(v.viewport().mapToGlobal(end)),Qt.NoButton,Qt.LeftButton,Qt.NoModifier)
    QApplication.sendEvent(v.viewport(),event)
    qtbot.mouseRelease(v.viewport(),Qt.LeftButton,pos=end)
    assert events and set(events[0])=={'0','1'}
