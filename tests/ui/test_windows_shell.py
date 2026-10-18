import sys
import pytest
from pathlib import Path
from PySide6.QtWidgets import QWidget


def read_unicode_shortcut(path):
    # WScript.Shell reads shortcut filenames through the system code page.
    # Read with IShellLinkW so the verifier also works on English Windows.
    import ctypes
    from project_manager.windows_shell import GUID, method, check
    ole=ctypes.WinDLL('ole32')
    ole.CoInitializeEx.argtypes=[ctypes.c_void_p,ctypes.c_ulong];ole.CoInitializeEx.restype=ctypes.c_long
    hr=ole.CoInitializeEx(None,2)
    if hr<0 and (hr & 0xffffffff)!=0x80010106:check(hr)
    link=ctypes.c_void_p();persist=ctypes.c_void_p()
    try:
        create=ole.CoCreateInstance
        create.argtypes=[ctypes.POINTER(GUID),ctypes.c_void_p,ctypes.c_ulong,ctypes.POINTER(GUID),ctypes.POINTER(ctypes.c_void_p)]
        create.restype=ctypes.c_long
        clsid=GUID.parse('00021401-0000-0000-C000-000000000046')
        iid=GUID.parse('000214F9-0000-0000-C000-000000000046')
        check(create(ctypes.byref(clsid),None,1,ctypes.byref(iid),ctypes.byref(link)))
        persist_iid=GUID.parse('0000010b-0000-0000-C000-000000000046')
        check(method(link,0,ctypes.POINTER(GUID),ctypes.POINTER(ctypes.c_void_p))(
            link,ctypes.byref(persist_iid),ctypes.byref(persist)))
        check(method(persist,5,ctypes.c_wchar_p,ctypes.c_ulong)(persist,str(path),0))
        result={}
        for name,index in (('target',3),('cwd',8),('args',10)):
            buffer=ctypes.create_unicode_buffer(32768)
            if name=='target':
                check(method(link,index,ctypes.c_wchar_p,ctypes.c_int,ctypes.c_void_p,ctypes.c_ulong)(
                    link,buffer,len(buffer),None,4))
            else:
                check(method(link,index,ctypes.c_wchar_p,ctypes.c_int)(link,buffer,len(buffer)))
            result[name]=buffer.value
        return result
    finally:
        if persist:method(persist,2)(persist)
        if link:method(link,2)(link)
        if hr>=0:ole.CoUninitialize()


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
    import subprocess
    from project_manager.windows_shell import register_window,window_properties,clear_window
    widget=QWidget();qtbot.addWidget(widget)
    executable=tmp_path/('프로그램 위치 '*5).strip()/'CodexManager.exe'
    executable.parent.mkdir();executable.write_bytes(b'fixture')
    icon=executable.with_suffix('.ico');icon.write_bytes(b'fixture')
    state=tmp_path/(('설정 저장 위치 '*3).strip()+'📁');state.mkdir()
    assert len(subprocess.list2cmdline([str(executable),'-s',str(state)]))>=260
    register_window(int(widget.winId()),executable,icon,state)
    values=window_properties(int(widget.winId()))
    try:
        assert values[5]=='Contentrium.CodexManager' and values[3]==str(icon)+',0'
        shortcut=Path(values[2].strip('"'))
        assert shortcut.is_file() and shortcut.parent==state
        link=read_unicode_shortcut(shortcut)
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
            link=read_unicode_shortcut(shortcut)
            assert Path(link['target'])==executable and Path(link['cwd'])==executable.parent
            assert link['args']==subprocess.list2cmdline(['-s',str(state)])
    finally:clear_window(int(widget.winId()))
