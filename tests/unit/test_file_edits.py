from pathlib import Path
import pytest
from project_manager.file_edits import edit_files,undo_files
from project_manager.journal import Journal


def env(tmp_path):
    root=tmp_path/'project';root.mkdir();source=root/'a.txt';source.write_text('original')
    dest=root/'dest';dest.mkdir();j=Journal(tmp_path/'state/journal.sqlite')
    return root,source,dest,j

@pytest.mark.parametrize('kind',['copy','move','delete','rename'])
def test_file_edit_and_undo(tmp_path,kind):
    root,source,dest,j=env(tmp_path)
    result=edit_files(kind,[source],dest,[root],[],j,tmp_path/'home',name='new.txt')
    assert result.state=='completed'
    op=j.restorable()[0];assert source.exists()==(kind=='copy')
    assert undo_files(op,j).state=='completed'
    assert source.read_text()=='original' and not j.restorable()
    assert not list(dest.iterdir())


def test_conflict_does_not_overwrite_and_changed_copy_blocks_undo(tmp_path):
    root,source,dest,j=env(tmp_path);(dest/source.name).write_text('existing')
    edit_files('copy',[source],dest,[root],[],j,tmp_path/'home')
    assert (dest/source.name).read_text()=='existing'
    copied=next(p for p in dest.iterdir() if p.name!=source.name);copied.write_text('edited later')
    with pytest.raises(ValueError):undo_files(j.restorable()[0],j)
    assert copied.read_text()=='edited later' and source.exists()


def test_root_and_recursive_targets_are_blocked(tmp_path):
    root,source,dest,j=env(tmp_path)
    for kind in ('delete','move','rename'):
        with pytest.raises(ValueError):edit_files(kind,[root],dest,[root],[],j,tmp_path/'home',name='other')
    folder=root/'folder';folder.mkdir();child=folder/'child';child.mkdir()
    with pytest.raises(ValueError):edit_files('copy',[folder],child,[root],[],j,tmp_path/'home')
    assert not j.pending()


def test_delete_restoration_never_overwrites_new_original(tmp_path):
    root,source,dest,j=env(tmp_path);edit_files('delete',[source],dest,[root],[],j,tmp_path/'home')
    source.write_text('new file')
    with pytest.raises(ValueError):undo_files(j.restorable()[0],j)
    assert source.read_text()=='new file'


def test_partial_copy_recovery_releases_lock_and_keeps_source(tmp_path,monkeypatch):
    import project_manager.file_edits as edits
    root,source,dest,j=env(tmp_path)
    def fail(src,target,stamp,cancelled=lambda:False):target.write_text('partial');raise OSError('disk full')
    monkeypatch.setattr(edits,'copy_item',fail)
    assert edit_files('copy',[source],dest,[root],[],j,tmp_path/'home').state=='needs_recovery'
    assert undo_files(j.pending()[0],j).state=='completed'
    assert not j.pending() and source.read_text()=='original' and not (dest/source.name).exists()


def test_racing_identical_target_is_never_deleted(tmp_path,monkeypatch):
    import project_manager.file_edits as edits
    root,source,dest,j=env(tmp_path);original=edits.copy_item
    def race(src,stage,stamp,cancelled=lambda:False):
        (dest/source.name).write_text('original');return original(src,stage,stamp)
    monkeypatch.setattr(edits,'copy_item',race)
    assert edit_files('copy',[source],dest,[root],[],j,tmp_path/'home').state=='needs_recovery'
    assert undo_files(j.pending()[0],j).state=='completed'
    assert (dest/source.name).read_text()=='original'


def test_cross_device_move_and_undo_preserve_tree(tmp_path,monkeypatch):
    import project_manager.file_edits as edits
    root,source,dest,j=env(tmp_path);folder=root/'nested';folder.mkdir();(folder/'empty').mkdir();(folder/'한글.txt').write_text('data')
    monkeypatch.setattr(edits,'same_device',lambda *_:False)
    assert edit_files('move',[folder],dest,[root],[],j,tmp_path/'home').state=='completed'
    assert not folder.exists() and (dest/'nested/한글.txt').read_text()=='data'
    op=j.restorable()[0];assert undo_files(op,j).state=='completed'
    assert (folder/'empty').is_dir() and (folder/'한글.txt').read_text()=='data'
    assert not (dest/'nested').exists()
    assert undo_files(op,j).state=='completed'


def test_cancel_before_start_does_not_mutate(tmp_path):
    root,source,dest,j=env(tmp_path)
    with pytest.raises(InterruptedError):edit_files('move',[source],dest,[root],[],j,tmp_path/'home',cancelled=lambda:True)
    assert source.exists() and not j.pending()
