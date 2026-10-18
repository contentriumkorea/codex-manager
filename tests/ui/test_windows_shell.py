import sys
import pytest
from pathlib import Path
from PySide6.QtWidgets import QWidget

@pytest.mark.skipif(sys.platform!='win32',reason='Windows property store')
def test_window_taskbar_identity_and_icon_are_explicit(qtbot,tmp_path,qapp):
    if qapp.platformName()!='windows':pytest.skip('A real Windows HWND is required; verified in native suite')
    from project_manager.windows_shell import register_window,window_properties,clear_window
    widget=QWidget();qtbot.addWidget(widget)
    executable=tmp_path/'한국어 앱'/'CodexManager.exe';icon=tmp_path/'한국어 앱'/'app.ico'
    register_window(int(widget.winId()),executable,icon,tmp_path/'설정')
    values=window_properties(int(widget.winId()))
    assert values[5]=='Contentrium.CodexManager'
    assert values[3]==str(icon)+',0'
    assert values[4]=='Codex Manager'
    assert str(executable) in values[2] and ' -s ' in values[2]

    clear_window(int(widget.winId()))
    assert all(v is None for v in window_properties(int(widget.winId())).values())


@pytest.mark.skipif(sys.platform!='win32',reason='Windows property store')
def test_long_relaunch_command_keeps_taskbar_identity_and_exact_state(qtbot,tmp_path,qapp):
    if qapp.platformName()!='windows':pytest.skip('A real Windows HWND is required')
    import json,subprocess
    from project_manager.windows_shell import register_window,window_properties,clear_window
    widget=QWidget();qtbot.addWidget(widget)
    executable=tmp_path/('프로그램 위치 '*5).strip()/'CodexManager.exe'
    executable.parent.mkdir();executable.write_bytes(b'fixture')
    icon=executable.with_suffix('.ico');icon.write_bytes(b'fixture')
    state=tmp_path/('설정 저장 위치 '*3).strip();state.mkdir()
    assert len(subprocess.list2cmdline([str(executable),'-s',str(state)]))>=260
    register_window(int(widget.winId()),executable,icon,state)
    values=window_properties(int(widget.winId()))
    try:
        assert values[5]=='Contentrium.CodexManager' and values[3]==str(icon)+',0'
        shortcut=Path(values[2].strip('"'))
        assert shortcut.is_file() and shortcut.parent==state
        literal="'"+str(shortcut).replace("'","''")+"'"
        code=f"[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); $s=(New-Object -ComObject WScript.Shell).CreateShortcut({literal}); @{{target=$s.TargetPath;args=$s.Arguments;cwd=$s.WorkingDirectory}} | ConvertTo-Json -Compress"
        result=subprocess.run(['powershell','-NoProfile','-Command',code],capture_output=True,text=True,encoding='utf-8',check=True,creationflags=subprocess.CREATE_NO_WINDOW)
        link=json.loads(result.stdout)
        assert Path(link['target'])==executable and Path(link['cwd'])==executable.parent
        assert link['args']==subprocess.list2cmdline(['-s',str(state)])
    finally:clear_window(int(widget.winId()))


@pytest.mark.skipif(sys.platform!='win32',reason='Windows property store')
@pytest.mark.parametrize('units,astral', [(255, False), (256, False), (258, False), (256, True), (258, True)])
def test_relaunch_at_windows_property_length_boundary(qtbot,tmp_path,qapp,units,astral):
    if qapp.platformName()!='windows':pytest.skip('A real Windows HWND is required')
    import subprocess
    from project_manager.windows_shell import register_window,window_properties,clear_window
    widget=QWidget();qtbot.addWidget(widget)
    executable=tmp_path/'한국어 앱'/'CodexManager.exe';executable.parent.mkdir();executable.write_bytes(b'fixture')
    icon=executable.with_suffix('.ico');icon.write_bytes(b'fixture')
    state=tmp_path/('설정' + ('📁' if astral else ''))
    base=subprocess.list2cmdline([str(executable),'-s',str(state)])
    state=state.with_name(state.name+'x'*(units-len(base.encode('utf-16-le'))//2));state.mkdir()
    command=subprocess.list2cmdline([str(executable),'-s',str(state)])
    assert len(command.encode('utf-16-le'))//2==units
    try:
        register_window(int(widget.winId()),executable,icon,state)
        values=window_properties(int(widget.winId()))
        assert values[5]=='Contentrium.CodexManager'
        if units<=255:assert values[2]==command
        else:
            shortcut=Path(values[2].strip('"'))
            assert shortcut==state/'taskbar-relaunch.lnk'
            assert subprocess.list2cmdline(['-s',str(state)]).encode('utf-16-le') in shortcut.read_bytes()
    finally:clear_window(int(widget.winId()))
