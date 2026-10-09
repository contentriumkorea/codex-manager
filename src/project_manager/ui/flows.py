"""Continue a confirmed transfer after safely attaching its inferred chats."""
from ..grouping import display_membership
from ..settings import read_settings
from ..operations import change_connections,transfer_project


def transfer_with_connections(adapter,project,destinations,journal,recovery,target,clean,inferred,progress,cancelled):
    if cancelled():raise InterruptedError('작업을 취소했습니다.')
    if inferred:
        snapshot=adapter.snapshot();current=next((p for p in snapshot.projects if p.id==project.id),None)
        membership=display_membership(snapshot,read_settings(adapter.home/'.codex-global-state.json'))
        if current!=project or any(membership.inferred.get(tid)!=project.id for tid in inferred):
            raise ValueError('프로젝트나 대화 연결이 바뀌었습니다. 새로고침한 뒤 다시 진행하세요.')
        result=change_connections(adapter,project,{tid:project.id for tid in inferred},journal)
        if result.state!='completed':return result
        if cancelled():raise InterruptedError('대화 연결을 저장한 뒤 취소했습니다. 파일은 이동하지 않았습니다.')
    return transfer_project(adapter,project,destinations,journal,recovery,target,clean,progress,cancelled)
