from pathlib import Path
from project_manager.cleanup import cleanup_export
from project_manager.restore_proof import verify_restored_bundle
import pytest


def test_missing_restore_proof_blocks_cleanup(tmp_path):
    from project_manager.bundles import export_project
    from test_bundles import sample
    p,s=sample(tmp_path);bundle=tmp_path/'bundle';export_project(p,s,bundle)
    class Adapter:
        home=tmp_path/'home'
        def ensure_write_allowed(self): pass
    with pytest.raises(ValueError,match='이어쓰기'):
        cleanup_export(p,bundle,Adapter(),None)
    assert (p.roots[0]/'테스트.txt').exists()
