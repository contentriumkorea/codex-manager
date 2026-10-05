import pytest
from project_manager.journal import Journal


def test_same_resource_operations_serialized(tmp_path):
    j=Journal(tmp_path/'journal.sqlite')
    j.begin('one',{'resources':['p']})
    with pytest.raises(RuntimeError): j.begin('two',{'resources':['p']})
    j.record('one','completed',{})
    j.begin('two',{'resources':['p']})
    assert [p['id'] for p in j.pending()]==['two']


def test_failed_operation_keeps_recovery_record(tmp_path):
    j=Journal(tmp_path/'journal.sqlite');j.begin('one',{'resources':['p']})
    j.record('one','needs_recovery',{'error':'interrupted'})
    assert Journal(tmp_path/'journal.sqlite').pending()[0]['state']=='needs_recovery'
