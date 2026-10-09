"""List-local shortcuts: text inputs retain native editing and undo."""
import uuid
from pathlib import Path
from PySide6.QtCore import Qt,QObject,QMimeData,QUrl
from PySide6.QtGui import QAction,QKeySequence
from PySide6.QtWidgets import QApplication,QInputDialog,QMessageBox,QListWidgetItem,QDialog
from ..file_edits import edit_files
from ..management import move_threads
from ..operations import recover_operation
from .dialogs import confirm
from .naming import ProjectPicker

MIME='application/x-codex-manager-cut'


class Shortcuts(QObject):
    def __init__(self,w):
        super().__init__(w);self.w=w;self.cut=None
        QApplication.clipboard().dataChanged.connect(self.clipboard_changed)
        self.actions={}
        for view in (w.project_list,w.thread_list,w.file_browser.view):
            for key,callback in [('Ctrl+C',lambda _checked=False,v=view:self.copy(v,False)),('Ctrl+X',lambda _checked=False,v=view:self.copy(v,True)),
                ('Ctrl+V',lambda _checked=False,v=view:self.paste(v)),('Ctrl+Z',self.undo),('Ctrl+A',view.selectAll),
                ('Delete',lambda _checked=False,v=view:self.delete(v)),('F2',lambda _checked=False,v=view:self.rename(v)),
                ('Return',lambda _checked=False,v=view:self.open(v)),('Enter',lambda _checked=False,v=view:self.open(v)),('Escape',self.cancel_cut)]:
                action=QAction(view);action.setShortcut(QKeySequence(key));action.setShortcutContext(Qt.WidgetShortcut);action.triggered.connect(callback);view.addAction(action)
                self.actions[(view,key)]=action
        w.file_browser.shortcut_handler=self
    def clipboard_changed(self):
        mime=QApplication.clipboard().mimeData()
        if self.cut and (mime is None or bytes(mime.data(MIME))!=self.cut['token'].encode()):self.cut=None
    def cancel_cut(self):
        self.cut=None;self.w.footer.setText('잘라내기를 취소했습니다. 원본은 유지됩니다.')
    def live(self):return self.w.mode in ('projects','conversations','unassigned')
    def ids(self,view):
        if not self.live():return ()
        return tuple(i.data(Qt.UserRole) for i in view.selectedItems())
    def paths(self):
        b=self.w.file_browser
        return tuple(Path(b.model.filePath(i)) for i in b.view.selectionModel().selectedRows(0))
    def busy(self):return bool(self.w.workers) or self.w.updater.requested
    def copy(self,view,cut=False):
        w=self.w;payload=None;mime=QMimeData()
        if cut and (not self.live() or self.busy()):return
        if view==w.file_browser.view:
            if not self.live():return
            paths=self.paths()
            if not paths:return
            mime.setUrls([QUrl.fromLocalFile(str(p)) for p in paths]);mime.setText('\n'.join(map(str,paths)))
            if cut:payload={'kind':'files','paths':paths}
        elif view==w.project_list and w.mode=='projects':
            projects=w.selected_projects()
            if not projects:return
            mime.setText('\n\n'.join(p.name+'\n'+'\n'.join(map(str,p.roots)) for p in projects))
            if cut:payload={'kind':'projects','projects':projects}
        else:
            ids=self.ids(view)
            if not ids:return
            threads=w.selected_threads(ids);mime.setText('\n'.join(t.title for t in threads))
            if cut:payload={'kind':'threads','ids':ids,'threads':threads}
        self.cut=None
        if payload:
            payload.update(token=uuid.uuid4().hex,home=w.adapter.home);mime.setData(MIME,payload['token'].encode())
        QApplication.clipboard().setMimeData(mime);self.cut=payload
        w.footer.setText('잘라내기 준비 · 대상 폴더/프로젝트에서 Ctrl+V · Esc 취소 · 아직 원본은 변경되지 않았습니다.' if cut else '복사했습니다. 파일은 Ctrl+V로 붙여넣고, 프로젝트·대화 이름은 입력창에 붙여넣을 수 있습니다.')
    def paste(self,view):
        w=self.w
        if self.busy() or not self.live():return
        self.clipboard_changed()
        clip=self.cut
        if clip and clip['home']!=w.adapter.home:
            QMessageBox.information(w,'붙여넣기','다른 저장소의 항목입니다. 원래 저장소에서 다시 선택하세요.');return
        if clip and clip['kind']=='threads':
            p=w.selected_project()
            if not p:
                d=ProjectPicker(w,w.snapshot.projects,'대화 붙여넣기','선택한 대화의 소속만 옮깁니다. 작업 파일 경로는 유지됩니다.')
                if d.exec()!=QDialog.Accepted:return
                p=next(p for p in w.snapshot.projects if p.id==d.project_id())
            if not confirm(w,'대화 옮기기',f'대화 {len(clip["ids"])}개 → {p.name}\n\n파일 위치는 유지하고 대화의 프로젝트 소속을 변경합니다.') or not w.guard_change():return
            def done(result):
                if result.state=='completed' and self.cut is clip:self.cancel_cut()
                w.show_result(result)
            w.run_job('대화 붙여넣기',lambda worker:move_threads(w.adapter,clip['threads'],p.id,w.journal),done);return
        if clip and clip['kind']=='projects':
            sources=clip['projects'];target=w.selected_project();ids={p.id for p in sources}
            if target and target.id in ids:target=None
            if not target:
                candidates=[p for p in w.snapshot.projects if p.id not in ids and p.roots]
                if not candidates:return
                d=ProjectPicker(w,candidates,'프로젝트 붙여넣기','선택한 프로젝트들을 합칠 대상을 선택하세요.')
                if d.exec()!=QDialog.Accepted:return
                target=next(p for p in candidates if p.id==d.project_id())
            def done(result):
                if result.state=='completed' and self.cut is clip:self.cancel_cut()
                w.show_result(result)
            w.transfer_projects(sources,target=target,done=done);return
        mime=QApplication.clipboard().mimeData()
        paths=clip['paths'] if clip and clip['kind']=='files' else tuple(Path(u.toLocalFile()) for u in mime.urls() if u.isLocalFile())
        if not paths:return
        destination=w.file_browser.current_path if view==w.file_browser.view else None
        if destination is None:
            p=w.selected_project()
            if p and p.roots:destination=p.roots[0]
        if destination is None:
            QMessageBox.information(w,'파일 붙여넣기','프로젝트의 대상 폴더를 열고 Ctrl+V를 누르세요.');return
        kind='move' if clip and clip['kind']=='files' else 'copy'
        self.files(kind,paths,destination,clip=clip)
    def files(self,kind,paths,destination,name=None,clip=None):
        w=self.w
        if self.busy():return
        if kind=='delete' and not confirm(w,'파일 삭제',f'{len(paths)}개 항목을 복구 보관소로 옮깁니다.\nCtrl+Z로 되돌릴 수 있으며 디스크 용량은 즉시 확보되지 않습니다.\n\n'+'\n'.join(map(str,paths))):return
        if kind=='move' and not confirm(w,'파일 이동','\n'.join(map(str,paths))+'\n→ '+str(destination)):return
        roots=tuple(r for p in w.snapshot.projects for r in p.roots);protected=tuple(t.cwd for t in w.snapshot.conversations)
        def done(result):
            if result.state=='completed' and clip and self.cut is clip:self.cancel_cut()
            w.show_result(result)
        w.run_job('파일 작업',lambda worker:edit_files(kind,paths,destination,roots,protected,w.journal,w.adapter.home,name=name,cancelled=worker.cancelled.is_set),done)
    def delete(self,view):
        w=self.w
        if not self.live() or self.busy():return
        if view==w.file_browser.view:
            paths=self.paths()
            if paths:self.files('delete',paths,w.file_browser.current_path)
        elif view==w.project_list and w.mode=='projects':w.delete_selected_project()
        else:w.delete_selected_threads(self.ids(view))
    def rename(self,view):
        w=self.w
        if not self.live() or self.busy():return
        if view==w.file_browser.view:
            paths=self.paths()
            if len(paths)!=1:return
            name,ok=QInputDialog.getText(w,'파일 이름 변경','확장자를 포함한 새 이름',text=paths[0].name)
            if ok:self.files('rename',paths,paths[0].parent,name=name)
        elif view==w.project_list and w.mode=='projects':w.rename_selected_project()
        else:w.rename_selected_thread(self.ids(view))
    def open(self,view):
        w=self.w
        if view==w.file_browser.view:w.file_browser.open_index(view.currentIndex())
        elif view==w.project_list:
            item=view.currentItem()
            if item:w.open_selected(item)
        else:
            item=view.currentItem()
            if item:w.show_transcript(item)
    def add_menu(self,menu,view):
        if not self.live():return
        menu.addSeparator()
        for label,key in [('복사','Ctrl+C'),('잘라내기','Ctrl+X'),('붙여넣기','Ctrl+V'),('실행 취소','Ctrl+Z')]:
            action=self.actions[(view,key)];action.setText(label+'\t'+key);menu.addAction(action)

    def help(self):
        QMessageBox.information(self.w,'기본 단축키','파일 목록\nCtrl+C 복사 · Ctrl+X 잘라내기 · Ctrl+V 현재 폴더에 붙여넣기\nDelete 복구 보관소로 삭제 · F2 파일 이름 변경\n\n프로젝트·대화 목록\nCtrl+C 이름/경로를 텍스트로 복사\nCtrl+X → 대상 프로젝트에서 Ctrl+V: 대화 소속 이동 / 프로젝트 합치기\nDelete 삭제 확인 · F2 이름 변경\n\n공통\nCtrl+Z 최근 복구 가능한 작업 되돌리기 (확인 후 적용)\nCtrl+A 현재 목록 전체 선택 · Enter 열기 · Esc 잘라내기 취소\nCtrl+F 검색 · F5 새로고침 · Alt+← 뒤로\n\n입력창에서는 복사·붙여넣기·실행 취소가 텍스트에 적용됩니다.\n파일 삭제·복사 취소 사본은 프로젝트 안의 .codex-manager-file-recovery에 보관되며 공간은 즉시 확보되지 않습니다.')

    def undo(self):
        w=self.w
        if self.busy() or not self.live():return
        operations=w.recovery_items()
        if not operations:w.footer.setText('되돌릴 수 있는 작업이 없습니다.');return
        # SQLite rowids reflect creation order across pending and completed records.
        with w.journal.connect() as db:order={row[0]:row[1] for row in db.execute('SELECT id,rowid FROM operations')}
        operation=max(operations,key=lambda op:order.get(op['id'],0));payload=operation['payload']
        label=payload.get('label') or {'connections':'프로젝트 이름·대화 연결 변경','rename-thread':'대화 제목 변경'}.get(payload['kind'],payload['kind'])
        if payload['kind'] not in ('file-edit','connections','rename-thread','rename-threads','management-delete'):
            w.set_mode('recovery');w.footer.setText('중단된 작업의 복구 항목을 확인하세요.');return
        if not confirm(w,'실행 취소',label+'\n\n이 작업을 이전 상태로 되돌립니다. 이후 변경된 파일과 기록은 덮어쓰지 않습니다.'):return
        if payload['kind']!='file-edit' and not w.guard_change():return
        w.run_job('실행 취소',lambda worker:recover_operation(operation['id'],w.adapter,w.journal),w.show_result)
