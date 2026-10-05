import json
from project_manager.codex.adapter import CodexAdapter
from project_manager.ui.window import MainWindow


def test_brand_and_updates_entry_keep_existing_settings(qtbot,tmp_path):
    state=tmp_path/'state';state.mkdir();(state/'settings.json').write_text(json.dumps({'startup_update_check':False,'custom':'preserved'}))
    w=MainWindow(CodexAdapter(tmp_path/'home',isolated=True),state,auto_refresh=False);qtbot.addWidget(w)
    assert w.windowTitle()=='Codex Manager'
    assert w.update_button.text()=='업데이트'
    assert not w.startup_update_check
    w.save_settings()
    assert json.loads((state/'settings.json').read_text())['custom']=='preserved'


def test_update_dialog_version_and_startup_option(qtbot,tmp_path):
    w=MainWindow(CodexAdapter(tmp_path/'home',isolated=True),tmp_path/'state',auto_refresh=False);qtbot.addWidget(w)
    d=w.open_updates(check=False);qtbot.addWidget(d)
    assert '0.2.0' in d.current.text()
    assert d.startup.isChecked()
    assert not d.install.isEnabled()
    d.startup.setChecked(False)
    assert not w.startup_update_check
