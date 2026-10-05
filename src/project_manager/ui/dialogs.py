from pathlib import Path
from PySide6.QtWidgets import (QDialog,QVBoxLayout,QLabel,QPlainTextEdit,QDialogButtonBox,
                              QComboBox,QLineEdit,QFormLayout,QCheckBox,QListWidget)
from ..models import Project


def confirm(parent,title,text):
    dialog=QDialog(parent);dialog.setWindowTitle(title);dialog.resize(660,440)
    layout=QVBoxLayout(dialog);layout.addWidget(QLabel(title))
    body=QPlainTextEdit();body.setReadOnly(True);body.setPlainText(text);layout.addWidget(body)
    buttons=QDialogButtonBox(QDialogButtonBox.Ok|QDialogButtonBox.Cancel)
    buttons.button(QDialogButtonBox.Ok).setText('실행')
    buttons.button(QDialogButtonBox.Cancel).setText('취소')
    buttons.accepted.connect(dialog.accept);buttons.rejected.connect(dialog.reject);layout.addWidget(buttons)
    return dialog.exec()==QDialog.Accepted


def confirm_transfer(parent,title,text):
    dialog=QDialog(parent);dialog.setWindowTitle(title);dialog.resize(680,460)
    layout=QVBoxLayout(dialog);body=QPlainTextEdit(text);body.setReadOnly(True);layout.addWidget(body)
    clean=QCheckBox('복사와 연결 검증 후 원본 폴더 정리');clean.setChecked(False);layout.addWidget(clean)
    layout.addWidget(QLabel('체크하지 않으면 원본 파일은 그대로 남습니다. 대화 기록은 유지합니다.'))
    buttons=QDialogButtonBox(QDialogButtonBox.Ok|QDialogButtonBox.Cancel)
    buttons.button(QDialogButtonBox.Ok).setText('실행');buttons.accepted.connect(dialog.accept);buttons.rejected.connect(dialog.reject);layout.addWidget(buttons)
    return clean.isChecked() if dialog.exec()==QDialog.Accepted else None


class ConnectionsDialog(QDialog):
    def __init__(self,parent,project,projects,threads):
        super().__init__(parent);self.setWindowTitle('연결 관리');self.resize(660,520)
        self.project=project;self.threads=threads
        layout=QVBoxLayout(self);form=QFormLayout()
        self.name=QLineEdit(project.name);form.addRow('프로젝트 이름',self.name)
        self.roots=QPlainTextEdit('\n'.join(str(r) for r in project.roots));self.roots.setMaximumHeight(130)
        form.addRow('연결 폴더 · 한 줄에 하나',self.roots);layout.addLayout(form)
        note=QLabel('폴더 연결 변경은 파일을 이동하거나 삭제하지 않습니다.\n현재 작업 경로를 함께 바꾸려면 프로젝트 옮기기를 사용하세요.')
        note.setObjectName('muted');note.setWordWrap(True);layout.addWidget(note)
        self.selected=QListWidget();self.selected.setSelectionMode(QListWidget.MultiSelection)
        for t in threads: self.selected.addItem(t.title)
        layout.addWidget(QLabel('소속을 바꿀 대화 선택'));layout.addWidget(self.selected)
        self.target=QComboBox();self.target.addItem('현재 소속 유지','unchanged');self.target.addItem('프로젝트에서 분리',None)
        for p in projects: self.target.addItem(p.name,p.id)
        layout.addWidget(self.target)
        buttons=QDialogButtonBox(QDialogButtonBox.Save|QDialogButtonBox.Cancel)
        buttons.accepted.connect(self.accept);buttons.rejected.connect(self.reject);layout.addWidget(buttons)

    def values(self):
        roots=tuple(Path(x.strip()) for x in self.roots.toPlainText().splitlines() if x.strip())
        if any(not p.is_absolute() for p in roots): raise ValueError('폴더는 전체 경로로 입력하세요.')
        if not self.name.text().strip(): raise ValueError('프로젝트 이름을 입력하세요.')
        tids=[self.threads[self.selected.row(item)].id for item in self.selected.selectedItems()]
        return Project(self.project.id,self.name.text().strip(),roots,self.project.legacy_ids),tids,self.target.currentData()
