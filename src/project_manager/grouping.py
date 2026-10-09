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
    known={t.id for t in snapshot.conversations}
    for t in snapshot.conversations:
        if t.project_id is not None:
            projects[t.id]=t.project_id if t.project_id in valid else None
            reasons[t.id]='explicit' if t.project_id in valid else 'missing_project';continue
        projects[t.id]=None
        if t.id in projectless:reasons[t.id]='projectless';continue
        if hosts.get(t.id,'local')!='local':reasons[t.id]='other_host';continue
        # A spawned agent belongs to its recorded parent even after that parent moves.
        # This is display-only; the original project_id and operation safeguards stay intact.
        if t.internal and t.parent_id in known:
            reasons[t.id]='internal_parent';continue
        cwd=path_key(t.cwd)
        matches=[(len(root),pid) for root,pid in roots if cwd==root or cwd.startswith(root+'\\')]
        if matches:
            length=max(x[0] for x in matches);pids={pid for n,pid in matches if n==length}
            if len(pids)==1:
                projects[t.id]=inferred[t.id]=pids.pop();reasons[t.id]='folder';continue
            reasons[t.id]='ambiguous'
        else:reasons[t.id]='outside'
    # Resolve parents before children so provisional folder matches never leak into
    # descendant memberships. An explicit stack also handles deep trees and cycles.
    threads={t.id:t for t in snapshot.conversations}
    resolved={tid for tid,reason in reasons.items() if reason not in ('folder','outside','internal_parent')}
    for start in threads:
        chain=[];positions={};tid=start
        while tid in threads and tid not in resolved:
            if tid in positions:
                for cyclic in chain[positions[tid]:]:
                    projects[cyclic]=None;inferred.pop(cyclic,None)
                    reasons[cyclic]='ambiguous';resolved.add(cyclic)
                break
            positions[tid]=len(chain);chain.append(tid);tid=threads[tid].parent_id
        for tid in reversed(chain):
            if tid in resolved:continue
            t=threads[tid];parent=projects.get(t.parent_id)
            if not t.internal and tid in inferred and parent and parent!=projects[tid]:
                projects[tid]=None;inferred.pop(tid);reasons[tid]='ambiguous'
            elif projects[tid] is None and reasons[tid] in ('outside','internal_parent') and parent:
                projects[tid]=inferred[tid]=parent;reasons[tid]='parent'
            resolved.add(tid)
    return DisplayMembership(projects,inferred,reasons)
