import hashlib,json,zipfile
from pathlib import Path
import pytest
from project_manager.updates import parse_release,stage_update,validate_install_plan,download_and_stage,check_release,store_prepared,load_prepared


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


def test_api_rate_limit_uses_public_release_assets(monkeypatch):
    from urllib.error import HTTPError
    seen=[]
    def read(url):
        seen.append(url)
        if 'api.github.com' in url: raise HTTPError(url,403,'rate limited',{},None)
        return {'version':'0.3.0','asset':'Codex-Manager-Windows-x64.zip'}
    monkeypatch.setattr('project_manager.updates.read_json_url',read)
    result=check_release('0.2.1')
    assert result.available and result.version=='0.3.0'
    assert seen[1].endswith('/releases/latest/download/update.json')


def test_truncated_download_is_retried_and_validated(tmp_path,monkeypatch):
    import io
    archive,metadata=package(tmp_path);payload=archive.read_bytes();calls=[]
    install=tmp_path/'app';install.mkdir();(install/'CodexManager.exe').write_bytes(b'old')
    monkeypatch.setattr('project_manager.updates.read_json_url',lambda u:metadata)
    def open_url(url): calls.append(url);return io.BytesIO(payload[:10] if len(calls)==1 else payload)
    monkeypatch.setattr('project_manager.updates.open_url',open_url)
    plan=download_and_stage(parse_release(release(),'0.2.1'),install,tmp_path/'state')
    assert len(calls)==2 and Path(plan['staged']).exists()
    assert not list(tmp_path.glob('.codex-manager-download-*'))


def test_cancelled_download_keeps_old_app_and_removes_download(tmp_path,monkeypatch):
    import io
    archive,metadata=package(tmp_path);install=tmp_path/'app';install.mkdir();(install/'CodexManager.exe').write_bytes(b'old')
    monkeypatch.setattr('project_manager.updates.read_json_url',lambda u:metadata)
    monkeypatch.setattr('project_manager.updates.open_url',lambda u:io.BytesIO(archive.read_bytes()))
    cancelled=[False]
    with pytest.raises(ValueError):
        download_and_stage(parse_release(release(),'0.2.1'),install,tmp_path/'state',progress=lambda *a:cancelled.__setitem__(0,True),cancelled=lambda:cancelled[0])
    assert (install/'CodexManager.exe').read_bytes()==b'old'
    assert not list(tmp_path.glob('.codex-manager-download-*'))


def test_prepared_cache_is_reusable_and_rejects_changed_file(tmp_path):
    archive,metadata=package(tmp_path);install=tmp_path/'app';install.mkdir();(install/'CodexManager.exe').write_bytes(b'old')
    release_info=parse_release(release(),'0.2.1');state=tmp_path/'state'
    plan=stage_update(archive,metadata,release_info,install,state);store_prepared(plan)
    assert load_prepared(release_info,install,state)
    (Path(plan['staged'])/'CodexManager.exe').write_bytes(b'tampered')
    assert load_prepared(release_info,install,state) is None
