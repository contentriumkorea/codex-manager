import hashlib,json,zipfile
from pathlib import Path
import pytest
from project_manager.updates import parse_release,stage_update,validate_install_plan


def release(version='0.3.0'):
    base=f'https://github.com/contentriumkorea/codex-manager/releases/download/v{version}/'
    return {'tag_name':'v'+version,'draft':False,'prerelease':False,'html_url':base.replace('/download/','/tag/').rstrip('/'),
            'body':'changes','assets':[{'name':name,'browser_download_url':base+name} for name in ('Codex-Manager-Windows-x64.zip','update.json')]}


def package(tmp_path,version='0.3.0',extra=None):
    payload={'CodexManager.exe':b'new-program','_internal/library.dll':b'new-library'}
    m={'version':version,'files':{p:hashlib.sha256(b).hexdigest() for p,b in payload.items()}}
    archive=tmp_path/'release.zip'
    with zipfile.ZipFile(archive,'w') as z:
        for p,b in payload.items(): z.writestr('CodexManager/'+p,b)
        z.writestr('CodexManager/install-manifest.json',json.dumps(m))
        if extra: z.writestr(extra,b'extra')
    return archive,{'version':version,'asset':'Codex-Manager-Windows-x64.zip','size':archive.stat().st_size,
                    'sha256':hashlib.sha256(archive.read_bytes()).hexdigest()}


def test_numeric_version_comparison_and_current_release():
    assert parse_release(release('0.10.0'),'0.9.0').available
    assert not parse_release(release('0.2.0'),'0.2.0').available
    assert not parse_release(release('0.1.0'),'0.2.0').available


@pytest.mark.parametrize('field,value',[('draft',True),('prerelease',True),('tag_name','garbage')])
def test_invalid_release_is_rejected(field,value):
    r=release();r[field]=value
    with pytest.raises(ValueError): parse_release(r,'0.2.0')


def test_asset_from_other_repository_is_rejected():
    r=release();r['assets'][0]['browser_download_url']='https://github.com/other/app/releases/download/v0.3.0/app.zip'
    with pytest.raises(ValueError): parse_release(r,'0.2.0')


def test_verified_update_staged_without_touching_old_program(tmp_path):
    archive,manifest=package(tmp_path);install=tmp_path/'installed';install.mkdir();(install/'CodexManager.exe').write_bytes(b'old')
    plan=stage_update(archive,manifest,parse_release(release(),'0.2.0'),install,tmp_path/'state',parent_pid=123)
    validate_install_plan(plan)
    assert (install/'CodexManager.exe').read_bytes()==b'old'
    assert (Path(plan['staged'])/'CodexManager.exe').read_bytes()==b'new-program'
    assert plan['state_dir']==str((tmp_path/'state').resolve())


@pytest.mark.parametrize('extra',['CodexManager/../escape.txt','Other/file.txt','CodexManager/unlisted.txt','CodexManager/_internal/CON'])
def test_unsafe_or_unlisted_archive_cannot_be_staged(tmp_path,extra):
    archive,manifest=package(tmp_path,extra=extra);install=tmp_path/'app';install.mkdir();(install/'CodexManager.exe').write_bytes(b'old')
    with pytest.raises(ValueError): stage_update(archive,manifest,parse_release(release(),'0.2.0'),install,tmp_path/'state',parent_pid=123)
    assert (install/'CodexManager.exe').read_bytes()==b'old'


def test_wrong_checksum_preserves_installation(tmp_path):
    archive,manifest=package(tmp_path);manifest['sha256']='0'*64
    install=tmp_path/'app';install.mkdir();(install/'CodexManager.exe').write_bytes(b'old')
    with pytest.raises(ValueError): stage_update(archive,manifest,parse_release(release(),'0.2.0'),install,tmp_path/'state',parent_pid=123)
    assert (install/'CodexManager.exe').read_bytes()==b'old'


def test_plan_cannot_replace_drive_root(tmp_path):
    with pytest.raises(ValueError): validate_install_plan({'install_dir':str(Path(tmp_path.anchor)), 'staged':str(tmp_path),'backup':str(tmp_path/'old'),'state_dir':str(tmp_path/'state')})


def test_state_link_rejected_before_staging(tmp_path,monkeypatch):
    archive,manifest=package(tmp_path);install=tmp_path/'app';install.mkdir();(install/'CodexManager.exe').write_bytes(b'old')
    state=tmp_path/'state';state.mkdir()
    monkeypatch.setattr('project_manager.updates.linked',lambda p:p==state)
    with pytest.raises(ValueError): stage_update(archive,manifest,parse_release(release(),'0.2.0'),install,state)
    assert not list(tmp_path.glob('.codex-manager-update-*'))
