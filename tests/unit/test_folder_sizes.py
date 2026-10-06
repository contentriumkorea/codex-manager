import os,subprocess
import pytest
from project_manager.folder_sizes import measure_folders


def test_streaming_size_avoids_double_counting_overlapping_roots(tmp_path):
    root=tmp_path/'files';child=root/'nested';child.mkdir(parents=True)
    (root/'a').write_bytes(b'123');(child/'b').write_bytes(b'12345')
    result=measure_folders((child,root,root))
    assert result.total==8 and result.files==2 and not result.skipped


def test_junction_and_missing_folder_are_visible_partial_results(tmp_path):
    root=tmp_path/'files';outside=tmp_path/'outside';root.mkdir();outside.mkdir()
    (root/'a').write_bytes(b'123');(outside/'large').write_bytes(b'x'*4096)
    link=root/'linked'
    if os.name=='nt':subprocess.run(['cmd','/c','mklink','/J',str(link),str(outside)],check=True,capture_output=True)
    else:link.symlink_to(outside,target_is_directory=True)
    result=measure_folders((root,tmp_path/'missing'))
    assert result.total==3 and result.files==1 and result.skipped==2 and len(result.warnings)==2
    assert measure_folders((link,)).total==0


def test_cancel_interrupts_between_entries(tmp_path):
    for i in range(20):(tmp_path/str(i)).write_bytes(b'x')
    checks=0
    def cancel():
        nonlocal checks
        checks+=1;return checks>5
    with pytest.raises(InterruptedError):measure_folders((tmp_path,),cancel)
