import os
from pathlib import Path
import pytest
from project_manager.files import scan_roots, copy_verified, validate_paths


def test_reject_nested_destination(tmp_path):
    assert validate_paths((tmp_path,),(tmp_path/'nested',),())


def test_copy_unicode_and_hash(tmp_path):
    src=tmp_path/'src';src.mkdir();(src/'한글.txt').write_text('안녕',encoding='utf-8')
    inv=scan_roots({'r':src},True)
    assert not inv.blockers
    dst=tmp_path/'dst'
    assert copy_verified(inv,{'r':src},{'r':dst},lambda *_:None,lambda:False).ok
    assert (dst/'한글.txt').read_text(encoding='utf-8')=='안녕'


def test_source_changes_block_transfer(tmp_path):
    src=tmp_path/'src';src.mkdir();f=src/'x';f.write_text('before')
    inv=scan_roots({'r':src},True);f.write_text('after')
    assert not copy_verified(inv,{'r':src},{'r':tmp_path/'dst'},lambda *_:None,lambda:False).ok


def test_overlapping_roots_block(tmp_path):
    nested=tmp_path/'child';nested.mkdir()
    assert scan_roots({'a':tmp_path,'b':nested},True).blockers


def test_hardlink_count_once(tmp_path):
    f=tmp_path/'x';f.write_text('abcd');os.link(f,tmp_path/'y')
    inv=scan_roots({'r':tmp_path},True)
    assert inv.logical_bytes==4
    assert len(inv.entries)==2


def test_collision_never_overwrites(tmp_path):
    src=tmp_path/'src';src.mkdir();(src/'x').write_text('new')
    dst=tmp_path/'dst';dst.mkdir();(dst/'x').write_text('old')
    assert not copy_verified(scan_roots({'r':src},True),{'r':src},{'r':dst},lambda *_:None,lambda:False).ok
    assert (dst/'x').read_text()=='old'
