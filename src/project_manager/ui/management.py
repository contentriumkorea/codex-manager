"""Explicit project and conversation management actions for the explorer."""
from pathlib import Path
from PySide6.QtCore import Qt
from PySide6.QtWidgets import (QDialog,QVBoxLayout,QLabel,QPlainTextEdit,QCheckBox,
    QDialogButtonBox,QInputDialog,QFileDialog,QMessageBox,QMenu,QListWidgetItem,QApplication)
from ..management import rename_project,rename_thread,rename_threads,move_threads,plan_delete,delete_items

from .naming import RenameDialog,ProjectPicker


class DeleteDialog(QDialog):
    def __init__(self,parent,plan):
        super().__init__(parent);self.setWindowTitle('프로젝트 삭제' if plan.project else '대화 삭제');self.resize(530,410)
        layout=QVBoxLayout(self)
        text=(f'프로젝트 «{plan.project.name}»와 대화 {len(plan.threads)}개' if plan.project else f'대화 {len(plan.threads)}개')+'를 Codex에서 삭제합니다.'
        label=QLabel(text);label.setWordWrap(True);layout.addWidget(label)
        listing=QPlainTextEdit();listing.setReadOnly(True)
        listing.setPlainText('\n'.join(t.title or t.id for t in plan.threads) or '연결된 대화 없음');layout.addWidget(listing)
        self.files=QCheckBox('연결된 실제 작업 폴더와 파일도 삭제');self.files.setChecked(False);self.files.setVisible(plan.project is not None);layout.addWidget(self.files)
        note=QLabel('작업 파일은 기본적으로 유지합니다. 삭제 전에 복구 사본을 만듭니다.\n삭제한 항목은 왼쪽 작업 복구에서 되살릴 수 있습니다.');note.setWordWrap(True);layout.addWidget(note)
        if plan.project:
            roots=QLabel('\n'.join(str(p) for p in plan.project.roots));roots.setWordWrap(True);layout.addWidget(roots)
        buttons=QDialogButtonBox(QDialogButtonBox.Ok|QDialogButtonBox.Cancel)
        buttons.button(QDialogButtonBox.Ok).setText('삭제');buttons.button(QDialogButtonBox.Cancel).setText('취소')
        buttons.button(QDialogButtonBox.Ok).setObjectName('danger')
        buttons.button(QDialogButtonBox.Cancel).setDefault(True)
        buttons.accepted.connect(self.accept);buttons.rejected.connect(self.reject);layout.addWidget(buttons)


class ManagementActions:
    def selected_thread_ids(self):
        if self.mode not in ('projects','unassigned','conversations'):return ()
        widget=self.project_list if self.mode in ('unassigned','conversations') else self.thread_list
        return tuple(item.data(Qt.UserRole) for item in widget.selectedItems())

    def selected_threads(self,ids=None):
        if self.mode not in ('projects','unassigned','conversations'):return ()
        ids=set(self.selected_thread_ids() if ids is None else ids)
        return tuple(t for t in self.snapshot.conversations if t.id in ids)

    def rename_selected_project(self):
        project=self.selected_project()
        if not project:return
        dialog=RenameDialog(self,[project.name],project=True)
        if dialog.exec()==QDialog.Accepted and self.guard_change():
            name=dialog.names()[0]
            self.run_job('프로젝트 이름 변경',lambda w:rename_project(self.adapter,project,name,self.journal),self.show_result)

    def rename_selected_thread(self,ids=None):
        threads=self.selected_threads(ids)
        if not threads:return
        dialog=RenameDialog(self,[t.title for t in threads])
        if dialog.exec()!=QDialog.Accepted or not self.guard_change():return
        names=dialog.names()
        if len(threads)==1:
            self.run_job('대화 제목 변경',lambda w:rename_thread(self.adapter,threads[0],names[0],self.journal),self.show_result)
        else:self.run_job('대화 제목 일괄 변경',lambda w:rename_threads(self.adapter,threads,names,self.journal),self.show_result)

    def rename_current(self):
        if self.mode=='projects':self.rename_selected_project()
        elif self.mode in ('unassigned','conversations'):self.rename_selected_thread()

    def delete_current(self):
        if self.mode=='projects':self.delete_selected_project()
        elif self.mode in ('unassigned','conversations'):self.delete_selected_threads()

    def copy_thread_titles(self,ids=None):
        threads=self.selected_threads(ids)
        QApplication.clipboard().setText('\n'.join(t.title for t in threads))
        self.footer.setText(f'대화 제목 {len(threads)}개를 복사했습니다.')


    def move_selected_threads(self,ids=None):
        threads=self.selected_threads(ids)
        if not threads:return
        projects=list(self.snapshot.projects)
        if not projects:QMessageBox.information(self,'대화 이동','대상 프로젝트가 없습니다.');return
        dialog=ProjectPicker(self,projects,'대화 프로젝트 이동',f'선택한 대화 {len(threads)}개의 소속을 변경합니다. 대화 내용과 작업 파일 위치는 유지됩니다.')
        if dialog.exec()==QDialog.Accepted and self.guard_change():
            target=dialog.project_id()
            self.run_job('대화 프로젝트 이동',lambda w:move_threads(self.adapter,threads,target,self.journal),self.show_result)

    def delete_selected_project(self):
        project=self.selected_project()
        if len(self.selected_projects())>1:
            QMessageBox.information(self,'프로젝트 선택','이 작업은 프로젝트 하나를 선택하세요. 여러 프로젝트 이동·합치기는 함께 처리할 수 있습니다.');return
        if not project or not self.guard_change():return
        try:plan=plan_delete(self.snapshot,project_id=project.id,inferred=tuple(t.id for t in self.folder_threads(project.id)))
        except ValueError as exc:QMessageBox.warning(self,'삭제 확인',str(exc));return
        self.confirm_delete(plan)

    def delete_selected_threads(self,ids=None):
        threads=self.selected_threads(ids)
        if not threads or not self.guard_change():return
        try:plan=plan_delete(self.snapshot,thread_ids=tuple(t.id for t in threads))
        except ValueError as exc:QMessageBox.warning(self,'삭제 확인',str(exc));return
        self.confirm_delete(plan)

    def confirm_delete(self,plan):
        dialog=DeleteDialog(self,plan)
        if dialog.exec()!=QDialog.Accepted:return
        files=dialog.files.isChecked();recovery=self.state_dir/'deleted-items'
        if files:
            folder=QFileDialog.getExistingDirectory(self,'삭제 전 복구 사본 저장 위치 (외장하드 권장)')
            if not folder:return
            recovery=Path(folder)/'Codex-Manager-Recovery'
        self.run_job('복구 사본 검증 후 삭제',lambda w:delete_items(self.adapter,plan,self.journal,recovery,files,w.progress.emit,w.cancelled.is_set),self.show_result)

    def thread_context_menu(self,point):
        item=self.thread_list.itemAt(point)
        if not item:return
        if not item.isSelected():self.thread_list.clearSelection();item.setSelected(True)
        ids=tuple(i.data(Qt.UserRole) for i in self.thread_list.selectedItems());menu=QMenu(self)
        preview=QListWidgetItem(item.text());preview.setData(Qt.UserRole,item.data(Qt.UserRole))
        menu.addAction('대화 읽기',lambda:self.show_transcript(preview))
        if ids and self.mode in ('projects','unassigned','conversations'):
            menu.addAction('제목 변경…' if len(ids)==1 else '제목 일괄 변경…',lambda:self.rename_selected_thread(ids))
            menu.addAction('제목 복사',lambda:self.copy_thread_titles(ids))
            menu.addAction(f'프로젝트로 이동… ({len(ids)}개)',lambda:self.move_selected_threads(ids))
            menu.addSeparator();menu.addAction(f'대화 삭제… ({len(ids)}개)',lambda:self.delete_selected_threads(ids))
        self.shortcuts.add_menu(menu,self.thread_list)
        menu.exec(self.thread_list.viewport().mapToGlobal(point))

    def sync_thread_actions(self):
        count=len(self.selected_thread_ids());self.thread_manage.setEnabled(count>0)
        self.thread_rename.setEnabled(count>0)
        self.thread_rename.setText('제목 변경…' if count<2 else '제목 일괄 변경…')
        self.rename_chat_button.setEnabled(count>0);self.rename_chat_button.setText('제목 변경' if count<2 else f'제목 변경 ({count})')
        self.selection_status.setText(f'{self.project_list.count():,}개 표시 · {len(self.project_list.selectedItems()):,}개 선택' if self.mode in ('unassigned','conversations') else f'프로젝트 {len(self.selected_projects())}개 선택 · 대화 {count:,}개 선택')
        self.thread_manage.setText('대화 관리');self.thread_manage.setToolTip(f'선택한 대화 {count}개: 이동·복사·삭제')

    def recovery_items(self):
        return [op for op in self.journal.pending()+self.journal.restorable() if Path(op['payload']['home']).resolve()==self.adapter.home.resolve()]
