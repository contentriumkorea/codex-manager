from PySide6.QtCore import Qt
from PySide6.QtWidgets import QDialogButtonBox
from project_manager.codex.adapter import CodexAdapter
from project_manager.models import Snapshot,Project,Conversation
from project_manager.ui.window import MainWindow
from project_manager.management import plan_delete
from project_manager.ui.management import DeleteDialog


def window(qtbot,tmp_path):
    root=tmp_path/'existing';root.mkdir()
    w=MainWindow(CodexAdapter(tmp_path/'home',isolated=True),tmp_path/'state',auto_refresh=False);qtbot.addWidget(w)
    w.refresh(Snapshot((Project('p','Existing',(root,)),),(),''));w.show();w.project_list.setCurrentRow(0)
    qtbot.waitUntil(lambda:w.size_scanner.worker is None,timeout=3000)
    return w


def test_permanent_choice_requires_acknowledgement_and_recovery_defaults_local(qtbot,tmp_path):
    w=window(qtbot,tmp_path);d=DeleteDialog(w,plan_delete(w.snapshot,project_id='p'));qtbot.addWidget(d)
    assert d.recovery_path==w.state_dir/'deleted-items'
    d.files.setChecked(True)
    assert d.buttons.button(QDialogButtonBox.Ok).isEnabled()
    d.permanent.setChecked(True)
    assert not d.buttons.button(QDialogButtonBox.Ok).isEnabled()
    d.acknowledge.setChecked(True)
    assert d.buttons.button(QDialogButtonBox.Ok).isEnabled()
    assert d.permanent.isChecked() and d.files.isChecked()


def test_import_preview_selects_multiple_and_replans_for_merge(qtbot,tmp_path):
    from project_manager.ui.migration import ImportDialog
    from project_manager.migration import Discovery,BundleInfo
    w=window(qtbot,tmp_path)
    def bundle(name):
        return BundleInfo(tmp_path/name,{'project':{'id':name,'name':name},'roots':[{'id':'root-01','original_path':'D:/'+name}],
                          'conversations':[],'inventory':[],'dependencies':[]})
    d=ImportDialog(w,Discovery((bundle('A'),bundle('B')),()));qtbot.addWidget(d)
    d.destination.setText(str(tmp_path/'new-location'));d.target.setCurrentIndex(1)
    plans=d.plans()
    assert len(plans)==2 and all(p.target_id=='p' for p in plans)
    assert all(p.destinations['root-01'].is_relative_to(w.snapshot.projects[0].roots[0]) for p in plans)
    d.list.topLevelItem(0).setCheckState(0,Qt.Unchecked)
    assert len(d.plans())==1
    d.list.topLevelItem(1).setCheckState(0,Qt.Unchecked)
    assert not d.buttons.button(QDialogButtonBox.Ok).isEnabled()


def test_project_actions_only_have_one_visible_context_row(qtbot,tmp_path):
    w=window(qtbot,tmp_path);w.selection_bar.activate(w.project_list)
    assert not w.project_actions.isVisible()
    assert not w.rename_chat_button.isVisible() and not w.thread_manage.isVisible()
    visible={k for k,b in w.selection_bar.buttons.items() if b.isVisible()}
    assert visible=={'backup','rename','delete'}
    assert w.selection_bar.more.isVisible() and w.import_button.isVisible()


def test_unselected_current_row_cannot_be_deleted(qtbot,tmp_path,monkeypatch):
    w=window(qtbot,tmp_path);p=w.snapshot.projects[0];q=Project('q','Other',(tmp_path/'other',))
    w.refresh(Snapshot((p,q),(),''))
    first=next(w.project_list.item(i) for i in range(w.project_list.count()) if w.project_list.item(i).data(Qt.UserRole)=='p')
    other=next(w.project_list.item(i) for i in range(w.project_list.count()) if w.project_list.item(i).data(Qt.UserRole)=='q')
    w.project_list.setCurrentItem(other);first.setSelected(True);other.setSelected(False)
    captured=[];monkeypatch.setattr(w,'confirm_delete',lambda plan:captured.append(plan.project.id))
    w.selection_bar.activate(w.project_list);w.selection_bar.dispatch('delete')
    assert captured==['p']
    qtbot.waitUntil(lambda:w.size_scanner.worker is None,timeout=3000)


def test_hidden_global_chat_filter_does_not_hide_project_conversations(qtbot,tmp_path):
    w=window(qtbot,tmp_path);p=w.snapshot.projects[0]
    t=Conversation('t','p',p.roots[0],(),False,None,'',False,'Active conversation')
    w.refresh(Snapshot((p,),(t,),''));w.set_mode('conversations');w.chat_status.setCurrentIndex(2)
    assert w.project_list.count()==0
    w.set_mode('projects');w.project_list.setCurrentRow(0)
    assert w.thread_list.count()==1
    qtbot.waitUntil(lambda:w.size_scanner.worker is None,timeout=3000)
