import pytest
from project_manager.models import Project,Conversation,Snapshot,OperationResult


def test_connection_failure_or_cancel_never_starts_transfer(tmp_path,monkeypatch):
    from project_manager.ui import flows
    p=Project('p','P',(tmp_path,));t=Conversation('t',None,tmp_path,(),False,None,'',False)
    class Adapter:
        home=tmp_path
        def snapshot(self):return Snapshot((p,),(t,),'')
    calls=[]
    monkeypatch.setattr(flows,'change_connections',lambda *args:OperationResult('needs_recovery','','','',errors=('failed',)))
    monkeypatch.setattr(flows,'transfer_project',lambda *args:calls.append(args))
    result=flows.transfer_with_connections(Adapter(),p,{},None,tmp_path,None,False,('t',),lambda *_:None,lambda:False)
    assert result.state=='needs_recovery' and not calls
    with pytest.raises(InterruptedError):flows.transfer_with_connections(Adapter(),p,{},None,tmp_path,None,False,('t',),lambda *_:None,lambda:True)
    assert not calls


def test_changed_membership_is_not_overwritten_by_transfer_prompt(tmp_path,monkeypatch):
    from project_manager.ui import flows
    p=Project('p','P',(tmp_path,));q=Project('q','Q',(tmp_path/'q',));t=Conversation('t','q',tmp_path,(),False,None,'',False)
    class Adapter:
        home=tmp_path
        def snapshot(self):return Snapshot((p,q),(t,),'')
    monkeypatch.setattr(flows,'change_connections',lambda *args:pytest.fail('Must not relink changed chat'))
    with pytest.raises(ValueError,match='새로고침'):flows.transfer_with_connections(Adapter(),p,{},None,tmp_path,None,False,('t',),lambda *_:None,lambda:False)


def test_successful_connections_continue_once_and_cancellation_after_link_stops(tmp_path,monkeypatch):
    from project_manager.ui import flows
    p=Project('p','P',(tmp_path,));t=Conversation('t',None,tmp_path,(),False,None,'',False)
    class Adapter:
        home=tmp_path
        def snapshot(self):return Snapshot((p,),(t,),'')
    events=[];cancelled=False
    monkeypatch.setattr(flows,'change_connections',lambda *args:(events.append('link'),OperationResult('completed','','',''))[1])
    monkeypatch.setattr(flows,'transfer_project',lambda *args:(events.append('transfer'),OperationResult('completed','','',''))[1])
    flows.transfer_with_connections(Adapter(),p,{},None,tmp_path,None,False,('t',),lambda *_:None,lambda:cancelled)
    assert events==['link','transfer']
    def connect(*args):
        nonlocal cancelled
        cancelled=True;return OperationResult('completed','','','')
    events.clear();monkeypatch.setattr(flows,'change_connections',connect)
    with pytest.raises(InterruptedError):flows.transfer_with_connections(Adapter(),p,{},None,tmp_path,None,False,('t',),lambda *_:None,lambda:cancelled)
    assert not events
