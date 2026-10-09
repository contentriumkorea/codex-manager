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
