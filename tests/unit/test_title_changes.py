import pytest
from project_manager.management import title_changes

def test_title_changes_literal_replacement_and_noops():
    assert title_changes(['A plan','A draft'],find='A',replacement='B',prefix='[work] ',suffix='!')==('[work] B plan!','[work] B draft!')
    assert title_changes(['same'])==('same',)

@pytest.mark.parametrize('names,kwargs',[(['x'],{'find':'x','replacement':''}),(['x'],{'prefix':'a'*256}),(['x'],{'suffix':'\nno'})])
def test_invalid_title_previews_are_rejected(names,kwargs):
    with pytest.raises(ValueError):title_changes(names,**kwargs)
