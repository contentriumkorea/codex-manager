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


def register_window(hwnd,executable,icon,state_dir):
    executable=Path(executable).resolve();icon=Path(icon).resolve();state_dir=Path(state_dir).resolve()
    values={2:subprocess.list2cmdline([str(executable),'-s',str(state_dir)]),3:str(icon)+',0',4:'Codex Manager',5:APP_ID}
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
