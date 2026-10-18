"""Explicit taskbar identity/relaunch icon through the Windows property store."""
import ctypes
import subprocess
import uuid
from pathlib import Path
from contextlib import contextmanager

APP_ID='Contentrium.CodexManager'

class GUID(ctypes.Structure):
    _fields_=[('bytes',ctypes.c_ubyte*16)]
    @classmethod
    def parse(cls,value):return cls.from_buffer_copy(uuid.UUID(value).bytes_le)

class KEY(ctypes.Structure):
    _fields_=[('fmtid',GUID),('pid',ctypes.c_uint32)]

class DATA(ctypes.Union):
    _fields_=[('text',ctypes.c_wchar_p),('alignment',ctypes.c_uint64),('padding',ctypes.c_ubyte*16)]

class VALUE(ctypes.Structure):
    _fields_=[('vt',ctypes.c_uint16),('reserved',ctypes.c_uint16*3),('data',DATA)]

FMT=GUID.parse('9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3')
IID=GUID.parse('886d8eeb-8cf2-4446-8d02-cdba1dbdcf99')


def method(store,index,*types):
    address=ctypes.cast(store,ctypes.POINTER(ctypes.POINTER(ctypes.c_void_p))).contents[index]
    return ctypes.WINFUNCTYPE(ctypes.c_long,ctypes.c_void_p,*types)(address)


def check(hr):
    if hr<0:raise OSError(f'Windows taskbar property failure: 0x{hr & 0xffffffff:08x}')


@contextmanager
def property_store(hwnd):
    shell=ctypes.WinDLL('shell32');get=shell.SHGetPropertyStoreForWindow
    get.argtypes=[ctypes.c_void_p,ctypes.POINTER(GUID),ctypes.POINTER(ctypes.c_void_p)];get.restype=ctypes.c_long
    store=ctypes.c_void_p();check(get(hwnd,ctypes.byref(IID),ctypes.byref(store)))
    try:yield store
    finally:method(store,2)(store)


def write_shortcut(path,executable,arguments,icon):
    # IShellLinkW/IPersistFile preserve Unicode paths; WScript.Shell can lose
    # non-ACP characters when saving the shortcut on Windows.
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
        for index,value in ((20,str(executable)),(11,arguments),(9,str(executable.parent)),(7,'Codex Manager')):
            check(method(link,index,ctypes.c_wchar_p)(link,value))
        check(method(link,17,ctypes.c_wchar_p,ctypes.c_int)(link,str(icon),0))
        persist_iid=GUID.parse('0000010b-0000-0000-C000-000000000046')
        check(method(link,0,ctypes.POINTER(GUID),ctypes.POINTER(ctypes.c_void_p))(
            link,ctypes.byref(persist_iid),ctypes.byref(persist)))
        check(method(persist,6,ctypes.c_wchar_p,ctypes.c_int)(persist,str(path),1))
    finally:
        if persist:method(persist,2)(persist)
        if link:method(link,2)(link)
        if hr>=0:ole.CoUninitialize()


def register_window(hwnd,executable,icon,state_dir):
    executable=Path(executable).resolve();icon=Path(icon).resolve();state_dir=Path(state_dir).resolve()
    command=subprocess.list2cmdline([str(executable),'-s',str(state_dir)])
    if len(command.encode('utf-16-le'))//2>255:
        # Windows rejects this property above 255 UTF-16 units, before MAX_PATH.
        # Count surrogate pairs too, so emoji paths use the same safe fallback.
        # ShellExecute also accepts a shortcut, whose arguments keep the full state path.
        shortcut=state_dir/'taskbar-relaunch.lnk';temporary=state_dir/(uuid.uuid4().hex+'.lnk')
        try:
            write_shortcut(temporary,executable,subprocess.list2cmdline(['-s',str(state_dir)]),icon)
            temporary.replace(shortcut)
        except OSError as exc:raise OSError('Taskbar relaunch shortcut could not be created.') from exc
        finally:temporary.unlink(missing_ok=True)
        command=subprocess.list2cmdline([str(shortcut)])
    values={2:command,3:str(icon)+',0',4:'Codex Manager',5:APP_ID}
    with property_store(hwnd) as store:
        setter=method(store,6,ctypes.POINTER(KEY),ctypes.POINTER(VALUE))
        # Relaunch information must precede the window AppUserModelID.
        for pid,text in values.items():
            key=KEY(FMT,pid);value=VALUE();value.vt=31;value.data.text=text
            try:check(setter(store,ctypes.byref(key),ctypes.byref(value)))
            except OSError as exc:
                for previous in values:
                    empty=VALUE();setter(store,ctypes.byref(KEY(FMT,previous)),ctypes.byref(empty))
                raise OSError(f'Window property {pid} ({len(text)} characters): {exc}') from exc
    return True


def window_properties(hwnd):
    values={};ole=ctypes.WinDLL('ole32');clear=ole.PropVariantClear;clear.argtypes=[ctypes.POINTER(VALUE)]
    with property_store(hwnd) as store:
        getter=method(store,5,ctypes.POINTER(KEY),ctypes.POINTER(VALUE))
        for pid in (2,3,4,5):
            key=KEY(FMT,pid);value=VALUE();check(getter(store,ctypes.byref(key),ctypes.byref(value)))
            try:values[pid]=value.data.text if value.vt==31 else None
            finally:clear(ctypes.byref(value))
    return values


def clear_window(hwnd):
    with property_store(hwnd) as store:
        setter=method(store,6,ctypes.POINTER(KEY),ctypes.POINTER(VALUE))
        for pid in (2,3,4,5):
            key=KEY(FMT,pid);value=VALUE();check(setter(store,ctypes.byref(key),ctypes.byref(value)))
