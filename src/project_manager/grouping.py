"""Read-only folder grouping; explicit operation membership remains authoritative."""
import ntpath
from dataclasses import dataclass
from .catalog import clean_path


@dataclass(frozen=True)
class DisplayMembership:
    projects:dict
    inferred:dict
    reasons:dict


def path_key(path):
    return ntpath.normcase(ntpath.normpath(str(clean_path(path)))).rstrip('\\')


def display_membership(snapshot,state):
    roots=[(path_key(r),p.id) for p in snapshot.projects for r in p.roots]
    valid={p.id for p in snapshot.projects};projects={};inferred={};reasons={}
    projectless=state.get('projectless-thread-ids',[])
    projectless=set(projectless) if isinstance(projectless,list) else set()
    hosts=state.get('thread-project-membership-host-ids',{})
    hosts=hosts if isinstance(hosts,dict) else {}
    for t in snapshot.conversations:
        if t.project_id is not None:
            projects[t.id]=t.project_id if t.project_id in valid else None
            reasons[t.id]='explicit' if t.project_id in valid else 'missing_project';continue
        projects[t.id]=None
        if t.id in projectless:reasons[t.id]='projectless';continue
        if hosts.get(t.id,'local')!='local':reasons[t.id]='other_host';continue
        cwd=path_key(t.cwd)
        matches=[(len(root),pid) for root,pid in roots if cwd==root or cwd.startswith(root+'\\')]
        if matches:
            length=max(x[0] for x in matches);pids={pid for n,pid in matches if n==length}
            if len(pids)==1:
                projects[t.id]=inferred[t.id]=pids.pop();reasons[t.id]='folder';continue
            reasons[t.id]='ambiguous'
        else:reasons[t.id]='outside'
    # Conflicting parent and folder evidence needs a choice rather than a guessed assignment.
    # Worktree children can have a different cwd; the recorded parent supplies display grouping.
    while True:
        changed=False
        for t in snapshot.conversations:
            parent=projects.get(t.parent_id)
            if t.id in inferred and parent and parent!=projects[t.id]:
                projects[t.id]=None;inferred.pop(t.id);reasons[t.id]='ambiguous';changed=True
            if projects[t.id] is None and reasons[t.id]=='outside' and projects.get(t.parent_id):
                projects[t.id]=inferred[t.id]=projects[t.parent_id];reasons[t.id]='parent';changed=True
        if not changed:break
    return DisplayMembership(projects,inferred,reasons)
