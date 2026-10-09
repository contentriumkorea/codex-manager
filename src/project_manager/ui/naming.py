from PySide6.QtCore import Qt
"""Validated names and literal batch changes with an exact preview."""
from PySide6.QtWidgets import (QDialog,QVBoxLayout,QFormLayout,QLabel,QLineEdit,
    QTreeWidget,QTreeWidgetItem,QDialogButtonBox,QHeaderView,QListWidget,QListWidgetItem)
from ..management import valid_name,title_changes


class RenameDialog(QDialog):
    def __init__(self,parent,names,project=False):
        super().__init__(parent);self.original=tuple(names);self.project=project;self._names=()
        self.setWindowTitle('프로젝트 이름 변경' if project else '대화 제목 변경' if len(names)==1 else '대화 제목 일괄 변경')
        self.resize(640,430);layout=QVBoxLayout(self)
        note=QLabel('실제 폴더명과 경로는 유지됩니다.' if project else '대화 내용과 프로젝트 연결은 유지됩니다. 제목 변경은 작업 복구에서 되돌릴 수 있습니다.')
        note.setWordWrap(True);layout.addWidget(note);form=QFormLayout();layout.addLayout(form)
        if len(names)==1:
            self.name=QLineEdit(names[0]);self.name.selectAll();form.addRow('새 이름' if project else '새 제목',self.name)
            self.name.textChanged.connect(self.update_preview)
        else:
            for attr,label in [('prefix','앞에 붙일 문구'),('find','찾을 문자열'),('replacement','바꿀 문자열'),('suffix','뒤에 붙일 문구')]:
                field=QLineEdit();setattr(self,attr,field);form.addRow(label,field);field.textChanged.connect(self.update_preview)
        self.preview=QTreeWidget();self.preview.setRootIsDecorated(False);self.preview.setHeaderLabels(['변경 전','변경 후']);self.preview.header().setSectionResizeMode(QHeaderView.Stretch);layout.addWidget(self.preview,1)
        self.status=QLabel();self.status.setWordWrap(True);layout.addWidget(self.status)
        self.buttons=QDialogButtonBox(QDialogButtonBox.Ok|QDialogButtonBox.Cancel)
        self.buttons.button(QDialogButtonBox.Ok).setText('변경');self.buttons.button(QDialogButtonBox.Cancel).setText('취소')
        self.buttons.accepted.connect(self.accept);self.buttons.rejected.connect(self.reject);layout.addWidget(self.buttons)
        self.update_preview()
    def update_preview(self):
        self.preview.clear();self._names=()
        try:
            names=(valid_name(self.name.text()),) if len(self.original)==1 else title_changes(self.original,self.find.text(),self.replacement.text(),self.prefix.text(),self.suffix.text())
            changed=sum(old!=new for old,new in zip(self.original,names));self._names=names
            for old,new in zip(self.original,names):
                item=QTreeWidgetItem([old,new]);item.setToolTip(0,old);item.setToolTip(1,new);self.preview.addTopLevelItem(item)
            self.status.setText(f'{changed}개 변경 · 이름은 1~256자' if changed else '변경할 이름을 입력하세요. · 이름은 1~256자')
            self.buttons.button(QDialogButtonBox.Ok).setEnabled(changed>0)
        except ValueError as exc:
            self.status.setText(str(exc));self.buttons.button(QDialogButtonBox.Ok).setEnabled(False)
    def names(self):return self._names
    def accept(self):
        self.update_preview()
        if self.buttons.button(QDialogButtonBox.Ok).isEnabled():super().accept()


class ProjectPicker(QDialog):
    def __init__(self,parent,projects,title,note):
        super().__init__(parent);self.projects=tuple(sorted(projects,key=lambda p:(p.name.casefold(),p.id)))
        self.setWindowTitle(title);self.resize(620,430);layout=QVBoxLayout(self)
        label=QLabel(note);label.setWordWrap(True);layout.addWidget(label)
        self.search=QLineEdit();self.search.setPlaceholderText('프로젝트 이름 또는 폴더 경로 검색');self.search.setClearButtonEnabled(True);layout.addWidget(self.search)
        self.list=QListWidget();layout.addWidget(self.list,1)
        self.buttons=QDialogButtonBox(QDialogButtonBox.Ok|QDialogButtonBox.Cancel)
        self.buttons.button(QDialogButtonBox.Ok).setText('선택');self.buttons.button(QDialogButtonBox.Cancel).setText('취소')
        self.buttons.accepted.connect(self.accept);self.buttons.rejected.connect(self.reject);layout.addWidget(self.buttons)
        self.search.textChanged.connect(self.refresh);self.list.currentItemChanged.connect(self.sync);self.list.itemDoubleClicked.connect(lambda *_:self.accept());self.refresh()
    def refresh(self):
        previous=self.project_id();query=self.search.text().casefold().strip();self.list.clear()
        for p in self.projects:
            paths=' · '.join(str(r) for r in p.roots);text=p.name+'\n'+paths
            if query and query not in text.casefold():continue
            item=QListWidgetItem(text);item.setData(Qt.UserRole,p.id);item.setToolTip(text+'\n'+p.id);self.list.addItem(item)
            if p.id==previous:self.list.setCurrentItem(item)
        self.sync()
    def sync(self,*_):self.buttons.button(QDialogButtonBox.Ok).setEnabled(self.project_id() is not None)
    def project_id(self):
        item=self.list.currentItem();return item.data(Qt.UserRole) if item else None
    def accept(self):
        if self.project_id() is not None:super().accept()
