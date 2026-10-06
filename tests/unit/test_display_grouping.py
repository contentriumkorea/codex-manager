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
