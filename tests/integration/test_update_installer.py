import hashlib,json,subprocess,os
from pathlib import Path
from project_manager.bundles import write_json


def test_failed_new_executable_restores_previous_folder_and_keeps_settings(tmp_path):
    old=tmp_path/'app';old.mkdir();(old/'CodexManager.exe').write_bytes(b'previous-program')
    (old/'user-note.txt').write_text('keep this')
    stage=tmp_path/'.codex-manager-update-test'/'CodexManager';stage.mkdir(parents=True)
    (stage/'CodexManager.exe').write_bytes(b'not an executable')
    write_json(stage/'install-manifest.json',{'version':'0.3.0','files':{'CodexManager.exe':hashlib.sha256(b'not an executable').hexdigest()}})
    state=tmp_path/'state';updates=state/'updates';updates.mkdir(parents=True)
    (state/'settings.json').write_text('{"home":"unchanged"}')
    plan={'id':'test','version':'0.3.0','install_dir':str(old),'staged':str(stage),'backup':str(tmp_path/'.codex-manager-previous-test'),
          'state_dir':str(state),'parent_pid':2147483647,'health':str(updates/'test.health.json'),'result':str(updates/'last-result.json')}
    write_json(updates/'plan.json',plan)
    script=Path(__file__).resolve().parents[2]/'src/project_manager/install_update.ps1'
    result=subprocess.run(['powershell','-NoProfile','-ExecutionPolicy','Bypass','-File',str(script),'-Plan',str(updates/'plan.json')],
                          capture_output=True,timeout=20,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
    assert result.returncode==1
    assert (old/'CodexManager.exe').read_bytes()==b'previous-program'
    assert (old/'user-note.txt').read_text()=='keep this'
    assert (state/'settings.json').read_text()=='{"home":"unchanged"}'
    assert json.loads((updates/'last-result.json').read_text(encoding='utf-8'))['state']=='failed'
    assert (tmp_path/'app.failed-test'/'CodexManager.exe').read_bytes()==b'not an executable'


def test_interrupted_first_rename_can_restore_previous_program(tmp_path):
    old=tmp_path/'app';backup=tmp_path/'.codex-manager-previous-interrupted';backup.mkdir()
    (backup/'CodexManager.exe').write_bytes(b'previous-program');(backup/'user-note.txt').write_text('preserve')
    stage=tmp_path/'.codex-manager-update-interrupted'/'CodexManager';stage.mkdir(parents=True)
    (stage/'CodexManager.exe').write_bytes(b'new')
    write_json(stage/'install-manifest.json',{'version':'0.3.0','files':{'CodexManager.exe':hashlib.sha256(b'new').hexdigest()}})
    state=tmp_path/'state';updates=state/'updates';updates.mkdir(parents=True)
    status=updates/'interrupted.status.json';write_json(status,{'id':'interrupted','phase':'replacing','install_dir':str(old),'backup':str(backup)})
    plan={'id':'interrupted','version':'0.3.0','install_dir':str(old),'staged':str(stage),'backup':str(backup),
          'state_dir':str(state),'parent_pid':2147483647,'health':str(updates/'interrupted.health.json'),
          'result':str(updates/'last-result.json'),'status':str(status)}
    write_json(updates/'plan.json',plan)
    script=Path(__file__).resolve().parents[2]/'src/project_manager/install_update.ps1'
    result=subprocess.run(['powershell','-NoProfile','-ExecutionPolicy','Bypass','-File',str(script),'-Plan',str(updates/'plan.json')],
                          capture_output=True,timeout=20,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
    assert result.returncode==0
    assert (old/'CodexManager.exe').read_bytes()==b'previous-program'
    assert (old/'user-note.txt').read_text()=='preserve'
    assert json.loads(status.read_text())['phase']=='recovered'
