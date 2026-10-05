from pathlib import Path
root=Path(SPECPATH).parent
a=Analysis([str(root/'packaging/launcher.py')],pathex=[str(root/'src')],binaries=[],
           datas=[(str(root/'src/project_manager/ui/theme.qss'),'project_manager/ui')],
           hiddenimports=[],hookspath=[],hooksconfig={},runtime_hooks=[],excludes=[],noarchive=False,optimize=0)
# Qt 6.11.2 uses Windows' unversioned ICU exports. A Poppler runtime on PATH
# supplies an incompatible ICU DLL; leave ICU resolution to Windows itself.
a.binaries=[entry for entry in a.binaries if not Path(entry[0]).name.lower().startswith('icu')]
pyz=PYZ(a.pure)
exe=EXE(pyz,a.scripts,[],exclude_binaries=True,name='ProjectManager',debug=False,
        bootloader_ignore_signals=False,strip=False,upx=False,console=False,
        disable_windowed_traceback=False,argv_emulation=False,target_arch=None,
        codesign_identity=None,entitlements_file=None)
coll=COLLECT(exe,a.binaries,a.datas,strip=False,upx=False,name='ProjectManager')
