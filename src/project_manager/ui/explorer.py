"""Project columns and a read-only, asynchronous file browser."""
from pathlib import Path
from PySide6.QtCore import Qt,Signal,QUrl,QSize
from PySide6.QtGui import QDesktopServices,QAction
from PySide6.QtWidgets import (QWidget,QVBoxLayout,QHBoxLayout,QPushButton,QLineEdit,QLabel,
    QComboBox,QTreeView,QTreeWidget,QTreeWidgetItem,QHeaderView,QAbstractItemView,
    QFileSystemModel,QMenu,QApplication,QStyle)


class BrowserItem(QTreeWidgetItem):
    # The window controller uses a single identity role across its list views.
    def data(self,column,role=None):return super().data(0,column) if role is None else super().data(column,role)
    def setData(self,*args):super().setData(*( (0,*args) if len(args)==2 else args))
    def text(self,column=0):return super().text(column)
    def setText(self,*args):super().setText(*( (0,*args) if len(args)==1 else args))
    def setToolTip(self,*args):super().setToolTip(*( (0,*args) if len(args)==1 else args))


class ProjectTable(QTreeWidget):
    def __init__(self):
        super().__init__();self.setRootIsDecorated(False);self.setUniformRowHeights(True)
        self.setAlternatingRowColors(False);self.setFrameShape(QTreeWidget.NoFrame)
        self.setSelectionBehavior(QAbstractItemView.SelectRows);self.setEditTriggers(QAbstractItemView.NoEditTriggers)
        self.setIconSize(QSize(20,20));self.setContextMenuPolicy(Qt.CustomContextMenu)
        self.setTextElideMode(Qt.ElideMiddle)
        self.configure(['이름','폴더 위치','대화','용량'])
    def configure(self,labels):
        self.setColumnCount(len(labels));self.setHeaderLabels(labels)
        for col in range(len(labels)):self.header().setSectionResizeMode(col,QHeaderView.Interactive)
        self.header().setStretchLastSection(True);self.setColumnWidth(0,220)
        if len(labels)>2:self.setColumnWidth(1,260);self.setColumnWidth(2,65)
    def add_record(self,values,key,icon=None):
        item=BrowserItem(values);item.setData(Qt.UserRole,key);item.setSizeHint(0,QSize(0,44))
        if icon:item.setIcon(0,icon)
        for col,text in enumerate(values):item.setToolTip(col,text)
        self.addTopLevelItem(item);return item
    def addItem(self,item):return self.add_record(item.text().split('\n',1),item.data(Qt.UserRole))
    def count(self):return self.topLevelItemCount()
    def item(self,index):return self.topLevelItem(index)
    def setCurrentRow(self,index):self.setCurrentItem(self.item(index))


class ProjectFiles(QFileSystemModel):
    def headerData(self,section,orientation,role=Qt.DisplayRole):
        if orientation==Qt.Horizontal and role==Qt.DisplayRole and 0<=section<4:return ['이름','크기','종류','수정한 날짜'][section]
        return super().headerData(section,orientation,role)
    def data(self,index,role=Qt.DisplayRole):
        if role==Qt.DisplayRole and index.column()==2:
            if self.isDir(index):return '폴더'
            suffix=self.fileInfo(index).suffix().upper();return suffix+' 파일' if suffix else '파일'
        return super().data(index,role)


class FileBrowser(QWidget):
    back_to_projects=Signal()
    def __init__(self,parent=None):
        super().__init__(parent);self.project_key=None;self.current_path=None;self.history=[];self.position=-1;self.roots=()
        layout=QVBoxLayout(self);layout.setContentsMargins(0,0,0,0);layout.setSpacing(10)
        bar=QHBoxLayout();self.back=QPushButton('←');self.back.setToolTip('뒤로');self.back.setFixedWidth(36);self.back.clicked.connect(self.go_back);bar.addWidget(self.back)
        self.up=QPushButton('↑');self.up.setToolTip('상위 폴더');self.up.setFixedWidth(36);self.up.clicked.connect(self.go_up);bar.addWidget(self.up)
        self.location=QLineEdit();self.location.setPlaceholderText('프로젝트 폴더 경로');self.location.returnPressed.connect(lambda:self.navigate(Path(self.location.text())));bar.addWidget(self.location,1)
        self.reveal=QPushButton('탐색기에서 열기');self.reveal.clicked.connect(self.reveal_current);bar.addWidget(self.reveal);layout.addLayout(bar)
        self.root_picker=QComboBox();self.root_picker.currentIndexChanged.connect(self.choose_root);layout.addWidget(self.root_picker)
        self.model=ProjectFiles(self);self.model.setReadOnly(True)
        self.view=QTreeView();self.view.setModel(self.model);self.view.setRootIsDecorated(False);self.view.setUniformRowHeights(True)
        self.view.setEditTriggers(QAbstractItemView.NoEditTriggers);self.view.setSelectionBehavior(QAbstractItemView.SelectRows);self.view.setSelectionMode(QAbstractItemView.ExtendedSelection)
        self.view.setSortingEnabled(True);self.view.sortByColumn(0,Qt.AscendingOrder);self.view.setAlternatingRowColors(False)
        self.view.setColumnWidth(0,280);self.view.setColumnWidth(1,95);self.view.setColumnWidth(2,110)
        self.view.doubleClicked.connect(self.open_index);self.view.setContextMenuPolicy(Qt.CustomContextMenu);self.view.customContextMenuRequested.connect(self.context_menu)
        layout.addWidget(self.view,1);self.status=QLabel('프로젝트를 열면 파일과 폴더가 표시됩니다.');self.status.setObjectName('muted');self.status.setWordWrap(True);layout.addWidget(self.status)
        self.model.directoryLoaded.connect(self.directory_loaded)
    def set_project(self,project):
        key=(project.id,tuple(str(p) for p in project.roots))
        if key==self.project_key:return
        self.project_key=key;self.roots=project.roots;self.history=[];self.position=-1;self.current_path=None
        self.root_picker.blockSignals(True);self.root_picker.clear()
        for root in self.roots:self.root_picker.addItem(str(root),str(root))
        self.root_picker.blockSignals(False);self.root_picker.setVisible(len(self.roots)>1)
        self.view.hide();self.reveal.setEnabled(False);self.location.clear()
        if self.roots:self.choose_root(0)
        else:self.status.setText('등록된 폴더가 없습니다. 프로젝트 설정에서 폴더를 추가하세요.')
    def choose_root(self,index):
        if 0<=index<len(self.roots):self.navigate(self.roots[index])
    def navigate(self,path,remember=True):
        path=Path(path)
        try:
            if not path.is_dir():raise ValueError('폴더를 찾을 수 없습니다. 외장 드라이브 연결이나 폴더 위치를 확인하세요.')
            resolved=path.resolve()
            if not any(resolved.is_relative_to(root.resolve()) for root in self.roots):raise ValueError('현재 프로젝트에 속한 폴더 경로를 입력하세요.')
        except (OSError,ValueError) as exc:
            self.status.setText(str(exc));self.location.setText(str(self.current_path or path));return False
        if remember and path!=self.current_path:
            self.history=self.history[:self.position+1]+[path];self.position=len(self.history)-1
        self.current_path=path;self.location.setText(str(path));self.location.setCursorPosition(0)
        self.view.setRootIndex(self.model.setRootPath(str(path)));self.view.show();self.reveal.setEnabled(True)
        self.status.setText('폴더는 두 번 클릭해 들어가고, 파일은 기본 프로그램으로 엽니다.');return True
    def directory_loaded(self,path):
        if self.current_path and Path(path)==self.current_path:
            count=self.model.rowCount(self.view.rootIndex())
            self.status.setText(f'{count:,}개 항목 · 파일을 두 번 클릭하면 기본 프로그램으로 엽니다.' if count else '이 폴더는 비어 있습니다.')
    def go_back(self):
        if self.position>0:
            if self.navigate(self.history[self.position-1],False):self.position-=1
        else:self.back_to_projects.emit()
    def go_up(self):
        if not self.current_path:return
        if any(self.current_path==r for r in self.roots):self.back_to_projects.emit()
        else:self.navigate(self.current_path.parent)
    def open_index(self,index):
        if not index.isValid():return
        path=Path(self.model.filePath(index))
        if path.is_dir():self.navigate(path)
        else:QDesktopServices.openUrl(QUrl.fromLocalFile(str(path)))
    def reveal_current(self):
        if self.current_path:QDesktopServices.openUrl(QUrl.fromLocalFile(str(self.current_path)))
    def context_menu(self,point):
        index=self.view.indexAt(point)
        if not index.isValid():return

        if not self.view.selectionModel().isSelected(index):self.view.setCurrentIndex(index)
        path=self.model.filePath(index);menu=QMenu(self)
        menu.addAction('열기',lambda:self.open_index(index))
        if hasattr(self,'shortcut_handler'):
            for title,key in [('복사','Ctrl+C'),('잘라내기','Ctrl+X'),('붙여넣기','Ctrl+V'),('이름 변경','F2'),('삭제','Delete'),('실행 취소','Ctrl+Z')]:
                action=self.shortcut_handler.actions[(self.view,key)];action.setText(title+'\t'+key);menu.addAction(action)
        menu.addAction('경로 복사',lambda:QApplication.clipboard().setText(path))
        menu.addAction('상위 폴더를 탐색기에서 열기',lambda:QDesktopServices.openUrl(QUrl.fromLocalFile(str(Path(path).parent))))
        menu.exec(self.view.viewport().mapToGlobal(point))
