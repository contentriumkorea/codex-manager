from project_manager.cleanup import remove_verified_files
from project_manager.files import scan_roots, copy_verified


def test_new_file_blocks_cleanup(tmp_path):
    src=tmp_path/'src';src.mkdir();(src/'a').write_text('a')
    dst=tmp_path/'dst';inv=scan_roots({'r':src},True)
    assert copy_verified(inv,{'r':src},{'r':dst},lambda *_:None,lambda:False).ok
    (src/'new').write_text('new')
    assert not remove_verified_files(inv,{'r':src},{'r':dst}).ok
    assert (src/'a').exists() and (src/'new').exists()


def test_tampered_target_blocks_cleanup(tmp_path):
    src=tmp_path/'src';src.mkdir();(src/'a').write_text('a')
    dst=tmp_path/'dst';inv=scan_roots({'r':src},True)
    copy_verified(inv,{'r':src},{'r':dst},lambda *_:None,lambda:False)
    (dst/'a').write_text('bad')
    assert not remove_verified_files(inv,{'r':src},{'r':dst}).ok
    assert (src/'a').exists()


def test_verified_source_cleanup(tmp_path):
    src=tmp_path/'src';src.mkdir();(src/'a').write_text('a')
    dst=tmp_path/'dst';inv=scan_roots({'r':src},True)
    copy_verified(inv,{'r':src},{'r':dst},lambda *_:None,lambda:False)
    assert remove_verified_files(inv,{'r':src},{'r':dst}).ok
    assert not src.exists() and (dst/'a').exists()
