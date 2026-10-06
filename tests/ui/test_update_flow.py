import sys,threading,time
from pathlib import Path
from project_manager.codex.adapter import CodexAdapter
from project_manager.ui.window import MainWindow
from project_manager.updates import Release


def window(qtbot,tmp_path,monkeypatch,download):
    install=tmp_path/'app';install.mkdir();(install/'CodexManager.exe').write_bytes(b'old')
    monkeypatch.setattr(sys,'frozen',True,raising=False);monkeypatch.setattr(sys,'executable',str(install/'CodexManager.exe'))
    monkeypatch.setattr('project_manager.ui.updates.download_and_stage',download)
    w=MainWindow(CodexAdapter(tmp_path/'home',isolated=True),tmp_path/'state',auto_refresh=False);qtbot.addWidget(w);w.show()
    return w


def test_detection_prefetches_and_ready_click_hands_off_once(qtbot,tmp_path,monkeypatch):
    downloads=[];launches=[]
    def download(*args): downloads.append(True);return {'version':'0.3.0'}
    w=window(qtbot,tmp_path,monkeypatch,download)
    monkeypatch.setattr('project_manager.ui.updates.launch_installer',lambda plan:launches.append(plan))
    w.update_checked(Release('0.3.0',True,'','','notes',''))
    qtbot.waitUntil(lambda:bool(downloads),timeout=2000)
    qtbot.waitUntil(lambda:w.updater.state=='ready',timeout=2000)
    d=w.open_updates(check=False)
    started=time.monotonic();d.install.click();d.install.click()
    assert time.monotonic()-started<.1
    qtbot.waitUntil(lambda:w.updater.state=='handoff',timeout=2000)
    assert len(downloads)==len(launches)==1


def test_clicked_while_downloading_waits_visibly_then_auto_installs(qtbot,tmp_path,monkeypatch):
    release_download=threading.Event();launches=[]
    def download(*args):
        assert release_download.wait(2);return {'version':'0.3.0'}
    w=window(qtbot,tmp_path,monkeypatch,download)
    monkeypatch.setattr('project_manager.ui.updates.launch_installer',lambda p:launches.append(p))
    w.update_checked(Release('0.3.0',True,'','','notes',''));d=w.open_updates(check=False)
    d.install.click();assert d.isVisible()
    assert w.updater.requested and not launches
    release_download.set();qtbot.waitUntil(lambda:len(launches)==1,timeout=3000)
    qtbot.waitUntil(lambda:w.updater.state=='handoff',timeout=2000)


def test_download_error_keeps_dialog_visible_and_retry_works(qtbot,tmp_path,monkeypatch):
    attempts=[]
    def download(*args):
        attempts.append(True)
        if len(attempts)==1: raise OSError('connection lost')
        return {'version':'0.3.0'}
    w=window(qtbot,tmp_path,monkeypatch,download)
    w.update_checked(Release('0.3.0',True,'','','notes',''));d=w.open_updates(check=False)
    qtbot.waitUntil(lambda:w.updater.state=='error',timeout=2000)
    assert d.isVisible() and 'connection lost' in d.status.text() and d.install.isEnabled()
    # Retry prepares again without requiring a second GitHub check.
    d.install.click();qtbot.waitUntil(lambda:len(attempts)==2,timeout=2000)
    w.updater.requested=False
    qtbot.waitUntil(lambda:w.updater._worker is None,timeout=2000)


def test_prepared_update_waits_for_data_job_without_requiring_second_click(qtbot,tmp_path,monkeypatch):
    w=window(qtbot,tmp_path,monkeypatch,lambda *a:{'version':'0.3.0'});launches=[]
    monkeypatch.setattr('project_manager.ui.updates.launch_installer',lambda p:launches.append(p))
    w.update_checked(Release('0.3.0',True,'','','notes',''))
    qtbot.waitUntil(lambda:w.updater.state=='ready',timeout=2000)
    w.workers.append('active-backup');d=w.open_updates(check=False);d.install.click()
    assert w.updater.state=='waiting' and not launches
    w.workers.clear();qtbot.waitUntil(lambda:len(launches)==1,timeout=2000)
    qtbot.waitUntil(lambda:w.updater.state=='handoff',timeout=2000)


def test_installer_start_failure_keeps_app_open_and_retry_enabled(qtbot,tmp_path,monkeypatch):
    w=window(qtbot,tmp_path,monkeypatch,lambda *a:{'version':'0.3.0'})
    def fail(plan): raise OSError('cannot start installer')
    monkeypatch.setattr('project_manager.ui.updates.launch_installer',fail)
    w.update_checked(Release('0.3.0',True,'','','notes',''));d=w.open_updates(check=False)
    qtbot.waitUntil(lambda:w.updater.state=='ready',timeout=2000)
    d.install.click();qtbot.waitUntil(lambda:w.updater.state=='error',timeout=2000)
    assert w.isVisible() and d.isVisible() and d.install.isEnabled()
    assert 'cannot start installer' in d.status.text()


def test_cancel_does_not_allow_retry_until_download_worker_stops(qtbot,tmp_path,monkeypatch):
    stop=threading.Event();attempts=[]
    def download(*args):
        attempts.append(True)
        assert stop.wait(3)
        return {'version':'0.3.0'}
    w=window(qtbot,tmp_path,monkeypatch,download)
    w.update_checked(Release('0.3.0',True,'','','notes',''));d=w.open_updates(check=False)
    qtbot.waitUntil(lambda:len(attempts)==1,timeout=2000)
    d.cancel_button.click()
    assert w.updater.state=='cancelling' and not d.install.isEnabled()
    w.updater.request_install()
    assert not w.updater.requested and len(attempts)==1
    stop.set();qtbot.waitUntil(lambda:w.updater.state=='cancelled',timeout=2000)
    assert d.install.isEnabled() and w.updater._worker is None


def test_withdrawn_release_clears_pending_install(qtbot,tmp_path,monkeypatch):
    stop=threading.Event();launches=[]
    def download(*args):
        assert stop.wait(3)
        return {'version':'0.3.0'}
    w=window(qtbot,tmp_path,monkeypatch,download)
    monkeypatch.setattr('project_manager.ui.updates.launch_installer',lambda p:launches.append(p))
    w.update_checked(Release('0.3.0',True,'','','notes',''));d=w.open_updates(check=False)
    d.install.click();w.update_checked(None);stop.set()
    qtbot.waitUntil(lambda:w.updater._worker is None,timeout=2000)
    assert not w.updater.requested and not launches and not d.install.isEnabled()
