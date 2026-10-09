"""Drag gestures route to journaled operations; Qt models never move data."""
import uuid
from pathlib import Path
from PySide6.QtCore import Qt,QObject,QEvent,QMimeData,QTimer
from PySide6.QtGui import QDrag,QPixmap,QPainter,QColor
from PySide6.QtWidgets import QTreeView,QListWidget,QFrame,QMessageBox
from .dialogs import confirm
from ..management import move_threads

MIME='application/x-codex-manager-drag'

class DragSource:
    def startDrag(self,actions):
        if hasattr(self,'drag_controller'):self.drag_controller.begin(self)

class FileDragView(DragSource,QTreeView):pass
class ChatDragList(DragSource,QListWidget):pass

class DragDrop(QObject):
    def __init__(self,w):
        super().__init__(w);self.w=w;self.payload=None;self.token=None;self.marks={};self.nav=None
        self.views=(w.project_list,w.thread_list,w.file_browser.view)
        for view in self.views:
            view.drag_controller=self;view.setDragEnabled(True);view.setAcceptDrops(True);view.viewport().setAcceptDrops(True)
            view.viewport().installEventFilter(self);view.setDropIndicatorShown(False)
            mark=QFrame(view.viewport());mark.setAttribute(Qt.WA_TransparentForMouseEvents);mark.setStyleSheet('background:rgba(90,140,230,35);border:2px solid #648ddd;border-radius:6px;');mark.hide();self.marks[view]=mark
        self.nav=next(b for b,m in w.nav_buttons if m=='projects');self.nav.setAcceptDrops(True);self.nav.installEventFilter(self)
        self.hover=QTimer(self);self.hover.setSingleShot(True);self.hover.setInterval(500);self.hover.timeout.connect(w.show_projects)

    def mime_for(self,view):
        w=self.w
        if not w.shortcuts.live() or w.shortcuts.busy():return None
        if view==w.file_browser.view:
            paths=w.shortcuts.paths()
            if not paths:return None
            payload={'kind':'files','paths':paths}
        elif view==w.project_list and w.mode=='projects':
            sources=w.selected_projects()
            if not sources:return None
            payload={'kind':'projects','sources':sources}
        else:
            threads=w.selected_threads(w.shortcuts.ids(view))
            if not threads:return None
            payload={'kind':'threads','threads':threads}
        self.token=uuid.uuid4().hex;self.payload=dict(payload,home=w.adapter.home)
        mime=QMimeData();mime.setData(MIME,self.token.encode());return mime

    def begin(self,view):
        mime=self.mime_for(view)
        if mime is None:return
        drag=QDrag(self.w);drag.setMimeData(mime)
        count=len(self.payload.get('sources',self.payload.get('threads',self.payload.get('paths',()))))
        pix=QPixmap(230,40);pix.fill(QColor('#eef4ff'));p=QPainter(pix);p.setPen(QColor('#243858'));p.drawText(pix.rect(),Qt.AlignCenter,f'{count}개 항목 · 대상에 끌어 놓기');p.end();drag.setPixmap(pix)
        try:drag.exec(Qt.CopyAction|Qt.MoveAction,Qt.MoveAction)
        finally:self.payload=None;self.token=None;self.clear();drag.deleteLater()

    def decode(self,mime):
        if mime.hasFormat(MIME):
            if not self.payload or bytes(mime.data(MIME))!=(self.token or '').encode() or self.payload['home']!=self.w.adapter.home:return None
            return self.payload
        urls=mime.urls()
        if urls and all(u.isLocalFile() for u in urls):return {'kind':'files','paths':tuple(Path(u.toLocalFile()) for u in urls),'external':True}
        return None

    def plan(self,mime,view,point,modifiers):
        w=self.w
        if not w.shortcuts.live() or w.shortcuts.busy():return None
        payload=self.decode(mime)
        if not payload:return None
        target=None;destination=None
        if view==w.project_list and w.mode=='projects':
            item=view.itemAt(point)
            if item:target=next((p for p in w.snapshot.projects if p.id==item.data(Qt.UserRole)),None)
            if target and target.roots:destination=target.roots[0]
        elif view==w.file_browser.view:
            index=view.indexAt(point)
            if index.isValid():
                candidate=Path(w.file_browser.model.filePath(index))
                if not candidate.is_dir():return None
                destination=candidate
            else:destination=w.file_browser.current_path
        if payload['kind']=='projects':
            if not target or not target.roots or any(p.id==target.id for p in payload['sources']):return None
            return dict(payload,target=target,label=f"프로젝트 {len(payload['sources'])}개 → {target.name}에 합치기",action='move')
        if payload['kind']=='threads':
            if not target or all(t.project_id==target.id for t in payload['threads']):return None
            return dict(payload,target=target,label=f"대화 {len(payload['threads'])}개 → {target.name}로 이동",action='move')
        if destination is None:return None
        paths=payload['paths'];action='copy' if payload.get('external') or modifiers & Qt.ControlModifier else 'move'
        if any(destination==p or destination.is_relative_to(p) for p in paths):return None
        if action=='move' and all(p.parent==destination for p in paths):return None
        return dict(payload,destination=destination,action=action,label=f"파일 {len(paths)}개 → {destination.name} {'복사' if action=='copy' else '이동'} · Ctrl: 복사")

    def clear(self):
        self.hover.stop()
        for mark in self.marks.values():mark.hide()

    def eventFilter(self,obj,event):
        kind=event.type()
        if kind not in (QEvent.DragEnter,QEvent.DragMove,QEvent.DragLeave,QEvent.Drop):return False
        if obj==self.nav:
            if kind in (QEvent.DragEnter,QEvent.DragMove):
                if self.w.shortcuts.live() and not self.w.shortcuts.busy() and self.decode(event.mimeData()):
                    event.acceptProposedAction()
                    if not self.hover.isActive():self.hover.start()
                else:event.ignore()
                return True
            if kind in (QEvent.DragLeave,QEvent.Drop):self.hover.stop();event.ignore();return True
            return False
        view=next((v for v in self.views if v.viewport()==obj),None)
        if view is None:return False
        if kind==QEvent.DragLeave:self.clear();return True
        if kind not in (QEvent.DragEnter,QEvent.DragMove,QEvent.Drop):return False
        plan=self.plan(event.mimeData(),view,event.position().toPoint(),event.modifiers())
        if not plan:self.clear();event.ignore();return True
        action=Qt.CopyAction if plan['action']=='copy' else Qt.MoveAction
        if not event.possibleActions() & action:event.ignore();return True
        event.setDropAction(action);event.accept()
        self.w.footer.setText(plan['label']+' · 놓으면 확인 · Esc 취소')
        if kind==QEvent.Drop:
            self.clear()
            # Finish the native drag loop before opening modal confirmation.
            QTimer.singleShot(0,lambda p=plan:self.perform(p))
        else:
            index=view.indexAt(event.position().toPoint());rect=view.visualRect(index) if index.isValid() else view.viewport().rect().adjusted(2,2,-2,-2)
            mark=self.marks[view];mark.setGeometry(rect);mark.show();mark.raise_()
        return True

    def perform(self,plan):
        w=self.w
        if not w.shortcuts.live() or w.shortcuts.busy() or plan.get('home',w.adapter.home)!=w.adapter.home:return
        if plan['kind']=='files':w.shortcuts.files(plan['action'],plan['paths'],plan['destination']);return
        if plan['kind']=='projects':w.transfer_projects(plan['sources'],target=plan['target']);return
        if not confirm(w,'대화 옮기기',plan['label']+'\n\n대화 제목과 내용, 작업 파일 위치는 유지합니다.') or not w.guard_change():return
        adapter=w.adapter;threads=plan['threads'];target=plan['target']
        w.run_job('대화 옮기기',lambda worker:move_threads(adapter,threads,target.id,w.journal),w.show_result)
