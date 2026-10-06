from pathlib import Path
from PySide6.QtCore import Qt
from PySide6.QtWidgets import QDialog,QVBoxLayout,QHBoxLayout,QLabel,QListWidget,QListWidgetItem,QPushButton,QFileDialog


class StoresDialog(QDialog):
    def __init__(self,window):
        super().__init__(window);self.window=window;self.setWindowTitle('Codex 저장소 관리');self.resize(760,460)
        layout=QVBoxLayout(self)
        note=QLabel('이 컴퓨터의 여러 Codex 저장소를 찾아 등록합니다.\n목록에서 저장소를 전환하면 각각의 프로젝트와 대화를 볼 수 있습니다.');note.setWordWrap(True);layout.addWidget(note)
        self.list=QListWidget();layout.addWidget(self.list,1)
        self.status=QLabel('기본 경로·이전에 등록한 위치·로컬 드라이브를 검색합니다.');self.status.setWordWrap(True);layout.addWidget(self.status)
        row=QHBoxLayout();layout.addLayout(row)
        for text,func in [('자동 검색',window.start_store_scan),('폴더 추가',window.choose_home),('추가 검색 위치',self.search),('이 저장소 사용',self.use)]:
            b=QPushButton(text);b.clicked.connect(lambda checked=False,f=func:f());row.addWidget(b)
        self.list.itemDoubleClicked.connect(lambda item:self.use())
    def refresh(self):
        selected=self.list.currentItem();old=selected.data(Qt.UserRole) if selected else str(self.window.adapter.home)
        self.list.clear()
        for home,info in self.window.stores.items():
            current='현재 저장소 · ' if home==self.window.adapter.home else ''
            status=f'프로젝트 {info.projects}개 · 대화 {info.chats:,}개' if info.available else '연결 확인 필요 · '+info.error
            item=QListWidgetItem(current+str(home)+'\n'+status);item.setData(Qt.UserRole,str(home));self.list.addItem(item)
            if str(home)==old:self.list.setCurrentItem(item)
    def search(self):
        folder=QFileDialog.getExistingDirectory(self,'저장소를 추가로 검색할 상위 폴더')
        if folder:self.window.start_store_scan(Path(folder))
    def use(self):
        item=self.list.currentItem()
        if item:self.window.store_picker.setCurrentIndex(self.window.store_picker.findData(item.data(Qt.UserRole)))
