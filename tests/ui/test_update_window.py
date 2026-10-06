import json
import pytest
from project_manager.codex.adapter import CodexAdapter
from project_manager.ui.window import MainWindow
from project_manager.version import VERSION


def test_brand_and_updates_entry_keep_existing_settings(qtbot,tmp_path):
    state=tmp_path/'state';state.mkdir();(state/'settings.json').write_text(json.dumps({'startup_update_check':False,'custom':'preserved'}))
    w=MainWindow(CodexAdapter(tmp_path/'home',isolated=True),state,auto_refresh=False);qtbot.addWidget(w)
    assert w.windowTitle()=='Codex Manager'
    assert w.update_button.text()=='업데이트'
    assert not w.startup_update_check
    w.save_settings()
    assert json.loads((state/'settings.json').read_text())['custom']=='preserved'


@pytest.mark.parametrize('saved',[[],None,{'backups':None},{'backups':[None,123],'home':[],'startup_update_check':[]}])
def test_malformed_settings_do_not_crash_and_original_is_preserved(qtbot,tmp_path,saved):
    state=tmp_path/'state';state.mkdir();text=json.dumps(saved)
    (state/'settings.json').write_text(text,encoding='utf-8')
    w=MainWindow(CodexAdapter(tmp_path/'home',isolated=True),state,auto_refresh=False);qtbot.addWidget(w)
    w.save_settings()
    assert any(p.read_text(encoding='utf-8')==text for p in state.glob('settings.invalid-*.json'))


def test_disconnected_external_backup_is_not_pruned(qtbot,tmp_path):
    state=tmp_path/'state';state.mkdir();offline=tmp_path/'unplugged-external-drive'/'backup'
    (state/'settings.json').write_text(json.dumps({'backups':[str(offline)]}),encoding='utf-8')
    w=MainWindow(CodexAdapter(tmp_path/'home',isolated=True),state,auto_refresh=False);qtbot.addWidget(w)
    w.save_settings()
    assert json.loads((state/'settings.json').read_text())['backups']==[str(offline)]


def test_update_dialog_version_and_startup_option(qtbot,tmp_path):
    w=MainWindow(CodexAdapter(tmp_path/'home',isolated=True),tmp_path/'state',auto_refresh=False);qtbot.addWidget(w)
    d=w.open_updates(check=False);qtbot.addWidget(d)
    assert VERSION in d.current.text()
    assert d.startup.isChecked()
    assert not d.install.isEnabled()
    d.startup.setChecked(False)
    assert not w.startup_update_check
