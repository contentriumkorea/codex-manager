import json,threading
from project_manager.models import Project,Snapshot
from project_manager.codex.adapter import CodexAdapter
from project_manager.ui.window import MainWindow
from project_manager.stores import Store,StoreScan


def test_detected_stores_and_old_paths_are_retained_after_switch(qtbot,tmp_path,monkeypatch):
    a=tmp_path/'first';b=tmp_path/'previous';a.mkdir();b.mkdir()
    for h in (a,b):(h/'.codex-global-state.json').write_text('{}')
    monkeypatch.setattr('project_manager.ui.window.discover_stores',lambda *args:StoreScan((Store(a,True),Store(b,True)),2,False))
    w=MainWindow(CodexAdapter(a,isolated=True),tmp_path/'state',auto_refresh=False);qtbot.addWidget(w)
    w.start_store_scan()
    qtbot.waitUntil(lambda:w._store_worker is None,timeout=3000)
    w.store_picker.setCurrentIndex(w.store_picker.findData(str(b)))
    qtbot.waitUntil(lambda:not w.workers,timeout=3000)
    assert w.adapter.home==b and set(w.stores)=={a,b}
    saved=json.loads((w.state_dir/'settings.json').read_text(encoding='utf-8'))
    assert saved['home']==str(b) and set(saved['homes'])=={str(a),str(b)}


def test_closing_cancels_background_store_scan_without_modal(qtbot,tmp_path,monkeypatch):
    entered=threading.Event()
    def search(known,roots,cancelled):
        entered.set()
        while not cancelled():threading.Event().wait(.01)
        raise InterruptedError()
    monkeypatch.setattr('project_manager.ui.window.discover_stores',search)
    w=MainWindow(CodexAdapter(tmp_path/'h',isolated=True),tmp_path/'s',auto_refresh=False);qtbot.addWidget(w);w.show();w.start_store_scan()
    qtbot.waitUntil(entered.is_set,timeout=2000)
    assert not w.workers
    w.close();qtbot.waitUntil(lambda:not w.isVisible(),timeout=3000)


def test_ready_update_waits_for_store_worker_cancellation_and_installs_once(qtbot,tmp_path,monkeypatch):
    import sys
    from project_manager.updates import Release
    install=tmp_path/'app';install.mkdir();(install/'CodexManager.exe').write_bytes(b'old')
    monkeypatch.setattr(sys,'frozen',True,raising=False);monkeypatch.setattr(sys,'executable',str(install/'CodexManager.exe'))
    monkeypatch.setattr('project_manager.ui.updates.download_and_stage',lambda *a:{'version':'0.3.0'})
    entered=threading.Event();finish=threading.Event();launches=[]
    def search(known,roots,cancelled):
        entered.set();assert finish.wait(3);return StoreScan((),0,False)
    monkeypatch.setattr('project_manager.ui.window.discover_stores',search)
    monkeypatch.setattr('project_manager.ui.updates.launch_installer',lambda plan:launches.append(plan))
    w=MainWindow(CodexAdapter(tmp_path/'h',isolated=True),tmp_path/'s',auto_refresh=False);qtbot.addWidget(w)
    w.start_store_scan();qtbot.waitUntil(entered.is_set,timeout=2000)
    w.update_checked(Release('0.3.0',True,'','','',''));qtbot.waitUntil(lambda:w.updater.state=='ready',timeout=2000)
    w.updater.request_install();assert w.updater.state=='waiting' and not launches
    finish.set();qtbot.waitUntil(lambda:w.updater.state=='handoff',timeout=3000)
    assert len(launches)==1 and w._store_worker is None


def test_failed_new_snapshot_keeps_old_store_and_old_project_together(qtbot,tmp_path,monkeypatch):
    a=tmp_path/'a';b=tmp_path/'b';a.mkdir();b.mkdir();errors=[]
    old=Snapshot((Project('same-id','Old project',(a,)),),(),'old')
    def snapshot(self,include_runtime=True):
        if self.home==b:raise OSError('cannot read new store')
        return old
    monkeypatch.setattr(CodexAdapter,'snapshot',snapshot)
    monkeypatch.setattr('project_manager.ui.window.store_info',lambda home,cancelled:Store(home,True))
    monkeypatch.setattr('project_manager.ui.window.QMessageBox.warning',lambda *args:errors.append(args[-1]))
    w=MainWindow(CodexAdapter(a,isolated=True),tmp_path/'s',auto_refresh=False);qtbot.addWidget(w);w.refresh(old);w.project_list.setCurrentRow(0)
    w.connect_store(b);qtbot.waitUntil(lambda:not w.workers,timeout=3000)
    assert errors and w.adapter.home==a and w.snapshot==old and w.selected_project().id=='same-id'
    assert b not in w.stores


def test_slow_store_verification_runs_off_the_gui_thread(qtbot,tmp_path,monkeypatch):
    import time
    entered=threading.Event();release=threading.Event()
    def info(home,cancelled):
        entered.set();assert release.wait(3);return Store(home,True)
    monkeypatch.setattr('project_manager.ui.window.store_info',info)
    w=MainWindow(CodexAdapter(tmp_path/'a',isolated=True),tmp_path/'s',auto_refresh=False);qtbot.addWidget(w)
    started=time.monotonic();w.connect_store(tmp_path/'b')
    assert time.monotonic()-started<.1
    qtbot.waitUntil(entered.is_set,timeout=2000)
    release.set();qtbot.waitUntil(lambda:not w.workers,timeout=3000)
    assert w.adapter.home==tmp_path/'b'
