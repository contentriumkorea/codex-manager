"""Exercise a separately named QA installer, update it, and uninstall it."""
import argparse,json,os,subprocess,sys,time,winreg
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];sys.path.insert(0,str(ROOT/'src'))
from project_manager.files import digest
from project_manager.bundles import write_json
from project_manager.version import VERSION,ASSET_NAME
from project_manager.updates import Release,stage_update,launch_installer

KEY=r'Software\Microsoft\Windows\CurrentVersion\Uninstall\Contentrium.CodexManager.QA_is1'


def registration():
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER,KEY) as key:
            return {name:winreg.QueryValueEx(key,name)[0] for name in ('DisplayName','DisplayVersion','InstallLocation','UninstallString')}
    except FileNotFoundError:return None


def wait_for(path,timeout=90):
    deadline=time.monotonic()+timeout
    while not path.exists() and time.monotonic()<deadline:time.sleep(.2)
    if not path.exists():raise TimeoutError(str(path))


def stop_owned(pid,exe):
    literal=lambda s:"'"+str(s).replace("'","''")+"'"
    script=f"$p=Get-Process -Id {int(pid)} -ErrorAction SilentlyContinue; if ($p) {{ if ($p.Path -ne {literal(exe)}) {{ throw 'Unexpected QA process' }}; Stop-Process -Id $p.Id }}"
    subprocess.run(['powershell','-NoProfile','-Command',script],check=True,creationflags=subprocess.CREATE_NO_WINDOW)


def verify(setup,root,release):
    setup=setup.resolve();root=root.resolve();release=release.resolve()
    marker=json.loads((setup.parent/'setup-qa.json').read_text(encoding='utf-8'))
    assert marker['test'] is True and digest(setup)==marker['sha256'],'Use a separately compiled --test setup.'
    assert not root.exists() and registration() is None,'Use a fresh QA install.'
    shortcut=Path(os.environ['APPDATA'])/'Microsoft/Windows/Start Menu/Programs/Codex Manager QA.lnk'
    assert not shortcut.exists()
    root.mkdir(parents=True);app=root/'installed';state=root/'state';state.mkdir();home=root/'home';home.mkdir()
    settings={'home':str(home),'custom':'retained QA settings','startup_update_check':False};write_json(state/'settings.json',settings)
    subprocess.run([str(setup),'/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/DIR='+str(app),'/LOG='+str(root/'install.log')],check=True,timeout=90)
    installed=registration();assert installed and installed['DisplayVersion']==VERSION and Path(installed['InstallLocation']).resolve()==app
    assert shortcut.exists() and (app/'unins000.exe').exists() and (app/'_internal').is_dir()
    quote=lambda s:"'"+str(s).replace("'","''")+"'"
    script=f"$s=(New-Object -ComObject WScript.Shell).CreateShortcut({quote(shortcut)}); [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); $s.TargetPath"
    link=subprocess.run(['powershell','-NoProfile','-Command',script],capture_output=True,text=True,encoding='utf-8',check=True,creationflags=subprocess.CREATE_NO_WINDOW)
    assert Path(link.stdout.strip()).resolve()==app/'CodexManager.exe'
    (app/'user-note.txt').write_text('keep user file')
    # The registered uninstaller must survive an in-app update, including new libraries.
    uninstall_hash=digest(app/'unins000.exe');log_hash=digest(app/'unins000.dat')
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER,KEY,0,winreg.KEY_SET_VALUE) as key:winreg.SetValueEx(key,'DisplayVersion',0,winreg.REG_SZ,'0.1.0-QA')
    metadata=json.loads((release/'update.json').read_text(encoding='utf-8'))
    plan=stage_update(release/ASSET_NAME,metadata,Release(VERSION,True,'','','',''),app,state,parent_pid=2147483647)
    sys.frozen=True;launch_installer(plan);result_path=Path(plan['result']);wait_for(result_path)
    result=json.loads(result_path.read_text(encoding='utf-8'));assert result['state']=='completed',result
    health=json.loads(Path(plan['health']).read_text(encoding='utf-8'))
    try:
        assert health['version']==VERSION and Path(health['install_dir']).resolve()==app
        assert health['taskbar_registered'] and not health['taskbar_error']
        assert digest(app/'unins000.exe')==uninstall_hash and digest(app/'unins000.dat')==log_hash
        assert registration()['DisplayVersion']==VERSION
    finally:stop_owned(health['pid'],app/'CodexManager.exe')
    (app/'_internal'/'qa-future-library.dll').write_bytes(b'future update library')
    subprocess.run([str(app/'unins000.exe'),'/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/LOG='+str(root/'uninstall.log')],check=True,timeout=60)
    deadline=time.monotonic()+30
    while (app/'unins000.exe').exists() and time.monotonic()<deadline:time.sleep(.2)
    assert not (app/'CodexManager.exe').exists() and not (app/'_internal').exists()
    assert not shortcut.exists() and registration() is None
    assert (Path(plan['backup'])/'user-note.txt').read_text()=='keep user file'
    current=json.loads((state/'settings.json').read_text(encoding='utf-8'))
    assert all(current.get(k)==v for k,v in settings.items())
    report={'state':'passed','version':VERSION,'install':True,'start_menu':True,'native_startup':True,'update_preserves_uninstaller':True,
            'uninstall':True,'settings_and_user_files_preserved':True,'root':str(root)}
    write_json(root/'verification.json',report);print(json.dumps(report),flush=True)


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--installer',type=Path,required=True);parser.add_argument('--root',type=Path,required=True);parser.add_argument('--release',type=Path,required=True)
    args=parser.parse_args();verify(args.installer,args.root,args.release)
