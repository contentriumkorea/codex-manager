"""File-manager layout; operations remain in the main window controller."""
from pathlib import Path
from PySide6.QtCore import Qt,QSize
from PySide6.QtGui import QIcon,QAction,QKeySequence
from PySide6.QtWidgets import (QWidget,QVBoxLayout,QHBoxLayout,QLabel,QPushButton,QLineEdit,
    QComboBox,QSplitter,QStackedWidget,QTabWidget,QListWidget,QAbstractItemView,QMenu)
from .explorer import ProjectTable,FileBrowser
from ..version import APP_NAME,VERSION


def label(text,kind='muted'):
    item=QLabel(text);item.setObjectName(kind);item.setWordWrap(True);return item


def build_layout(w):
    w.resize(1420,860);w.setMinimumSize(1060,680)
    base=QWidget();w.setCentralWidget(base);outer=QHBoxLayout(base);outer.setContentsMargins(0,0,0,0);outer.setSpacing(0)
    sidebar=QWidget();sidebar.setObjectName('sidebar');sidebar.setFixedWidth(194)
    nav=QVBoxLayout(sidebar);nav.setContentsMargins(16,22,16,18);nav.setSpacing(8)
    logo=QLabel();logo.setPixmap(QIcon(str(Path(__file__).parent/'app-icon.ico')).pixmap(40,40));nav.addWidget(logo)
    nav.addWidget(label(APP_NAME,'brand'));nav.addWidget(label('파일과 대화를 한곳에서'));nav.addSpacing(26)
    nav.addWidget(label('내 작업','section'));w.nav_buttons=[]
    for text,mode in [('모든 프로젝트','projects'),('분류할 대화','unassigned'),('백업','backups'),('작업 복구','recovery')]:
        button=QPushButton(text);button.setObjectName('nav');button.setCheckable(True);button.clicked.connect(lambda _,m=mode:w.set_mode(m));nav.addWidget(button);w.nav_buttons.append((button,mode))
    nav.addStretch();nav.addWidget(label('Codex 데이터 위치','section'))
    w.store_picker=QComboBox();w.store_picker.setMinimumContentsLength(12);w.store_picker.setSizeAdjustPolicy(QComboBox.AdjustToMinimumContentsLengthWithIcon);w.store_picker.currentIndexChanged.connect(w.switch_store);nav.addWidget(w.store_picker)
    w.connection=label('');w.connection.setMaximumHeight(36);nav.addWidget(w.connection)
    stores=QPushButton('다른 위치 찾기');stores.clicked.connect(w.open_stores);nav.addWidget(stores)
    w.update_button=QPushButton('업데이트');w.update_button.clicked.connect(lambda:w.open_updates());nav.addWidget(w.update_button)
    nav.addWidget(label('v'+VERSION+' · CONTENTRIUM'));outer.addWidget(sidebar)
    content=QVBoxLayout();content.setContentsMargins(24,23,24,14);content.setSpacing(13);outer.addLayout(content,1)
    top=QHBoxLayout();w.home_button=QPushButton('프로젝트');w.home_button.setObjectName('crumb');w.home_button.clicked.connect(w.show_projects);top.addWidget(w.home_button)
    w.heading=label('모든 프로젝트','heading');top.addWidget(w.heading);top.addStretch()
    w.search=QLineEdit();w.search.setPlaceholderText('이름, 폴더, 대화 검색');w.search.setClearButtonEnabled(True);w.search.setMaximumWidth(250);w.search.textChanged.connect(w.filter_list);top.addWidget(w.search)
    refresh=QPushButton('새로고침');refresh.clicked.connect(w.reload);top.addWidget(refresh);content.addLayout(top)
    w.subtitle=label('프로젝트를 두 번 클릭해 파일을 살펴보세요.');content.addWidget(w.subtitle)
    toolbar=QHBoxLayout();toolbar.setSpacing(7);w.project_actions=QWidget();buttons=QHBoxLayout(w.project_actions);buttons.setContentsMargins(0,0,0,0);buttons.setSpacing(7)
    w.buttons={};w.open_button=QPushButton('열기');w.open_button.clicked.connect(w.open_project);buttons.addWidget(w.open_button)
    for title,key,handler in [('옮기기','move',w.move),('합치기','merge',w.merge),('백업 만들기','backup',w.backup)]:
        button=QPushButton(title);button.clicked.connect(handler);buttons.addWidget(button);w.buttons[key]=button
    w.more=QPushButton('더 보기');w.more_menu=QMenu(w.more)
    for title,key,handler in [('이름 변경','rename',w.rename_selected_project),('프로젝트 설정','links',w.connections),('프로젝트 삭제','delete',w.delete_selected_project),('백업 후 원본 정리','cleanup',w.export_cleanup)]:
        action=w.more_menu.addAction(title,handler);w.buttons[key]=action
    w.more.setMenu(w.more_menu);buttons.addWidget(w.more);toolbar.addWidget(w.project_actions);toolbar.addStretch()
    w.sort=QComboBox();w.sort.addItems(['용량 큰 순','이름 순','용량 작은 순']);w.sort.setFixedWidth(120);w.sort.currentIndexChanged.connect(w.filter_list);toolbar.addWidget(w.sort)
    w.import_button=QPushButton('백업 가져오기');w.import_button.clicked.connect(w.open_backup);toolbar.addWidget(w.import_button);content.addLayout(toolbar)
    w.move_button=w.buttons['move'];w.cleanup_button=w.buttons['cleanup']
    w.split=QSplitter(Qt.Horizontal);content.addWidget(w.split,1)
    center=QWidget();center_layout=QVBoxLayout(center);center_layout.setContentsMargins(0,0,0,0);center_layout.setSpacing(0)
    w.browser_stack=QStackedWidget();w.project_list=ProjectTable();w.project_list.currentItemChanged.connect(w.show_detail)
    w.project_list.itemDoubleClicked.connect(w.open_selected);w.project_list.customContextMenuRequested.connect(w.project_context_menu)
    w.project_list.itemSelectionChanged.connect(lambda:w.show_detail() if w.mode=='unassigned' else None)
    w.browser_stack.addWidget(w.project_list);w.file_browser=FileBrowser();w.file_browser.back_to_projects.connect(w.show_projects);w.browser_stack.addWidget(w.file_browser)
    center_layout.addWidget(w.browser_stack,1);w.empty_state=label('불러오는 중입니다.','empty');w.empty_state.setAlignment(Qt.AlignCenter);center_layout.addWidget(w.empty_state)
    w.split.addWidget(center)
    detail=QWidget();detail.setObjectName('details');detail.setMinimumWidth(280);dl=QVBoxLayout(detail);dl.setContentsMargins(20,15,12,12);dl.setSpacing(12)
    w.detail_title=label('프로젝트를 선택하세요','detailTitle');dl.addWidget(w.detail_title)
    w.folder_size=label('');dl.addWidget(w.folder_size)
    w.detail_tabs=QTabWidget();dl.addWidget(w.detail_tabs,1)
    w.thread_list=QListWidget();w.thread_list.setWordWrap(True);w.thread_list.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff);w.thread_list.setSelectionMode(QAbstractItemView.ExtendedSelection);w.thread_list.itemDoubleClicked.connect(w.show_transcript)
    w.detail_tabs.addTab(w.thread_list,'대화')
    w.thread_list.setContextMenuPolicy(Qt.CustomContextMenu);w.thread_list.customContextMenuRequested.connect(w.thread_context_menu)
    w.folder_list=QListWidget();w.folder_list.setWordWrap(True);w.folder_list.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff);w.folder_list.itemDoubleClicked.connect(w.open_root)
    w.detail_tabs.addTab(w.folder_list,'폴더 위치')
    w.thread_manage=QPushButton('대화 관리');thread_menu=QMenu(w.thread_manage)
    w.thread_rename=thread_menu.addAction('제목 변경…',lambda:w.rename_selected_thread())
    thread_menu.addAction('프로젝트로 이동…',lambda:w.move_selected_threads())
    thread_menu.addSeparator();thread_menu.addAction('선택한 대화 삭제…',lambda:w.delete_selected_threads())
    w.thread_manage.setMenu(thread_menu);dl.addWidget(w.thread_manage);w.thread_list.itemSelectionChanged.connect(w.sync_thread_actions)
    w.detail_status=label('목록에서 프로젝트를 선택하면 대화와 폴더를 확인할 수 있습니다.');dl.addWidget(w.detail_status)
    for attr,title,handler in [('bundle_action','이 백업 복원하기',w.restore),('proof_action','복원 상태 확인',w.verify_restore),('recovery_action','선택한 항목 복구',w.show_recovery),('assign_action','선택한 대화 프로젝트 이동',lambda:w.move_selected_threads(tuple(i.data(Qt.UserRole) for i in w.project_list.selectedItems()))),('confirm_links','대화 연결 확인',w.connect_folder_threads)]:
        button=QPushButton(title);button.clicked.connect(handler);button.hide();setattr(w,attr,button);dl.addWidget(button)
    w.split.addWidget(detail);w.split.setSizes([860,310]);w.split.setChildrenCollapsible(False)
    bottom=QHBoxLayout();w.size_status=label('');w.size_status.setWordWrap(False);bottom.addWidget(w.size_status);bottom.addStretch();w.help_button=QPushButton('사용 안내');w.help_button.clicked.connect(w.show_help);bottom.addWidget(w.help_button);content.addLayout(bottom)
    w.footer=label('프로젝트를 두 번 클릭하면 파일이 열립니다.');content.addWidget(w.footer)
    for sequence,callback in [('Ctrl+F',lambda:w.search.setFocus()),('F5',w.reload),('Alt+Left',w.navigate_back)]:
        action=QAction(w);action.setShortcut(QKeySequence(sequence));action.triggered.connect(callback);w.addAction(action)
    for widget,rename,remove in [(w.project_list,lambda:w.rename_selected_project() if w.mode=='projects' else None,lambda:w.delete_selected_project() if w.mode=='projects' else None),(w.thread_list,lambda:w.rename_selected_thread(),lambda:w.delete_selected_threads())]:
        for sequence,callback in [('F2',rename),('Delete',remove)]:
            action=QAction(widget);action.setShortcut(QKeySequence(sequence));action.setShortcutContext(Qt.WidgetShortcut);action.triggered.connect(callback);widget.addAction(action)
