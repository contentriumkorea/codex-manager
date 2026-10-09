from pathlib import Path
from project_manager.models import Project,Conversation,Snapshot
from project_manager.grouping import display_membership


def chat(tid,cwd,pid=None,parent=None):return Conversation(tid,pid,Path(cwd),(),False,parent,'',False)


def test_folder_membership_matches_case_longest_root_and_preserves_explicit():
    ps=(Project('a','A',(Path('D:/work'),)),Project('b','B',(Path('D:/work/nested'),)))
    ts=(chat('t','d:/WORK/nested/sub'),chat('x','D:/work/nested','a'),chat('outside','D:/worker'))
    result=display_membership(Snapshot(ps,ts,''),{})
    assert result.projects=={'t':'b','x':'a','outside':None}
    assert result.inferred=={'t':'b'}


def test_ambiguous_explicit_projectless_and_foreign_host_are_not_guessed():
    ps=(Project('a','A',(Path('D:/work'),)),Project('b','B',(Path('D:/work'),)))
    ts=(chat('ambiguous','D:/work'),chat('none','D:/work'),chat('remote','D:/work'))
    result=display_membership(Snapshot(ps,ts,''),{'projectless-thread-ids':['none'],'thread-project-membership-host-ids':{'remote':'remote'}})
    assert all(pid is None for pid in result.projects.values()) and not result.inferred
    assert result.reasons['ambiguous']=='ambiguous' and result.reasons['none']=='projectless'


def test_subagent_inherits_display_group_and_its_source_record_stays_unassigned():
    p=Project('p','P',(Path('D:/work'),));ts=(chat('child','D:/isolated',parent='parent'),chat('parent','D:/work'))
    result=display_membership(Snapshot((p,),ts,''),{})
    assert result.projects['child']=='p' and ts[0].project_id is None


def test_known_subagent_uses_parent_project_over_old_folder():
    from dataclasses import replace
    ps=(Project('old','Old',(Path('D:/old'),)),Project('new','New',(Path('D:/new'),)))
    ts=(replace(chat('child','D:/old',parent='parent'),internal=True),chat('parent','D:/new','new'))
    result=display_membership(Snapshot(ps,ts,''),{})
    assert result.projects['child']=='new'
    assert result.reasons['child']=='parent'
    assert ts[0].project_id is None


def test_internal_parent_chain_is_order_independent_and_respects_opt_out():
    from dataclasses import replace
    p=Project('p','P',(Path('D:/new'),))
    ts=(replace(chat('child','D:/old',parent='middle'),internal=True),
        replace(chat('middle','D:/old',parent='parent'),internal=True),chat('parent','D:/new','p'))
    for rows in (ts,tuple(reversed(ts))):
        assert display_membership(Snapshot((p,),rows,''),{}).projects['child']=='p'
    result=display_membership(Snapshot((p,),ts,''),{'projectless-thread-ids':['middle']})
    assert result.projects['middle'] is None and result.projects['child'] is None


def test_internal_child_does_not_inherit_provisional_ambiguous_parent():
    from dataclasses import replace
    ps=(Project('a','A',(Path('D:/a'),)),Project('b','B',(Path('D:/b'),)))
    ts=(replace(chat('child','D:/old',parent='middle'),internal=True),
        chat('middle','D:/b',parent='parent'),chat('parent','D:/a','a'))
    for rows in (ts,tuple(reversed(ts))):
        result=display_membership(Snapshot(ps,rows,''),{})
        assert result.projects['middle'] is None and result.projects['child'] is None


def test_internal_parent_cycle_stays_unassigned():
    from dataclasses import replace
    ts=(replace(chat('a','D:/work',parent='b'),internal=True),replace(chat('b','D:/work',parent='a'),internal=True))
    result=display_membership(Snapshot((Project('p','P',(Path('D:/work'),)),),ts,''),{})
    assert result.projects=={'a':None,'b':None}
