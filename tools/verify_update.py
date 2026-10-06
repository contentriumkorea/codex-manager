"""Run the packaged updater in a fresh disposable directory, preserving originals."""
import argparse,json,sys,time
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'src'))
from project_manager.bundles import write_json
from project_manager.files import digest
from project_manager.updates import Release,stage_update,launch_installer
from project_manager.version import VERSION,ASSET_NAME


def verify(release_dir,root):
    root=root.resolve()
    if root.exists(): raise FileExistsError('Use a new isolated verification directory.')
    root.mkdir(parents=True)
    app=root/'프로그램 폴더';app.mkdir();(app/'CodexManager.exe').write_bytes(b'previous program')
    (app/'user-note.txt').write_text('retained user file',encoding='utf-8')
    write_json(app/'install-manifest.json',{'version':'0.1.0','files':{'CodexManager.exe':digest(app/'CodexManager.exe')}})
    state=root/'상태 폴더';home=root/'isolated-home';home.mkdir();state.mkdir()
    settings={'home':str(home),'custom':'retained settings','backups':[],'startup_update_check':False}
    write_json(state/'settings.json',settings)
    metadata=json.loads((release_dir/'update.json').read_text(encoding='utf-8'))
    release=Release(VERSION,True,'','','','')
    plan=stage_update(release_dir/ASSET_NAME,metadata,release,app,state,parent_pid=2147483647)
    # Exercise the production launcher, including trusted helper copy and cwd.
    sys.frozen=True
    started=time.monotonic()
    launch_installer(plan)
    ready_seconds=time.monotonic()-started
    result=Path(plan['result']);deadline=time.monotonic()+90
    while not result.exists() and time.monotonic()<deadline: time.sleep(.25)
    if not result.exists(): raise RuntimeError('Installer did not finish.')
    record=json.loads(result.read_text(encoding='utf-8'))
    if record['state']!='completed': raise RuntimeError(record)
    health=json.loads(Path(plan['health']).read_text(encoding='utf-8'))
    try:
        assert health['version']==VERSION and Path(health['install_dir']).resolve()==app
        assert digest(app/'CodexManager.exe')==json.loads((app/'install-manifest.json').read_text(encoding='utf-8'))['files']['CodexManager.exe']
        backup=Path(plan['backup'])
        assert (backup/'user-note.txt').read_text(encoding='utf-8')=='retained user file'
        assert not (backup/'CodexManager.exe').exists()
        assert json.loads((state/'settings.json').read_text(encoding='utf-8'))==settings
        assert list((state/'updates').glob('recover-*.cmd'))
        print(json.dumps({'state':'passed','version':VERSION,'root':str(root),'ready_seconds':round(ready_seconds,3),'total_seconds':round(time.monotonic()-started,3),'health':health,'result':record}))
    finally:
        # This PID belongs to the verified fresh application under this owned root.
        import ctypes
        kernel=ctypes.WinDLL('kernel32',use_last_error=True)
        kernel.OpenProcess.restype=ctypes.c_void_p
        kernel.TerminateProcess.argtypes=[ctypes.c_void_p,ctypes.c_uint]
        kernel.CloseHandle.argtypes=[ctypes.c_void_p]
        handle=kernel.OpenProcess(1,False,health['pid'])
        if handle:
            kernel.TerminateProcess(handle,0);kernel.CloseHandle(handle)


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--release',type=Path,required=True);parser.add_argument('--root',type=Path,required=True)
    args=parser.parse_args();verify(args.release,args.root)
