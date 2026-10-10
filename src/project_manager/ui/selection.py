"""Actions follow the last active list, even after a toolbar button takes focus."""
from PySide6.QtCore import Qt, QEvent
from PySide6.QtWidgets import QWidget, QHBoxLayout, QLabel, QPushButton


class SelectionBar(QWidget):
    def __init__(self, w):
        super().__init__(w); self.w = w; self.source = w.project_list; self.setObjectName('selectionBar'); self.setAttribute(Qt.WA_StyledBackground, True)
        row = QHBoxLayout(self); row.setContentsMargins(12, 7, 12, 7); row.setSpacing(6)
        self.summary = QLabel(); row.addWidget(self.summary); row.addStretch(); self.buttons = {}
        for key, title in [('move', '옮기기'), ('merge', '합치기'), ('backup', '백업'), ('rename', '이름 변경'), ('copy', '복사'), ('cut', '잘라내기'), ('paste', '붙여넣기'), ('delete', '삭제'), ('clear', '선택 해제')]:
            button = QPushButton(title); button.clicked.connect(lambda _=False, k=key: self.dispatch(k))
            row.addWidget(button); self.buttons[key] = button
        for view in (w.project_list, w.thread_list, w.file_browser.view):
            view.installEventFilter(self); view.viewport().installEventFilter(self)
            view.selectionModel().selectionChanged.connect(lambda *_args, v=view: self.selection_changed(v))
        self.hide()
        w.detail_tabs.currentChanged.connect(self.sync)

    def eventFilter(self, obj, event):
        if event.type() in (QEvent.FocusIn, QEvent.MouseButtonPress):
            for view in (self.w.project_list, self.w.thread_list, self.w.file_browser.view):
                if obj in (view, view.viewport()): self.activate(view); break
        return False

    def selection_changed(self, view):
        if self.source is view or view.hasFocus(): self.activate(view)
        elif view is self.w.project_list and self.w.browser_stack.currentWidget() is self.w.project_list: self.activate(view)

    def activate(self, view):
        self.source = view; self.sync()

    def selection(self):
        w = self.w
        if w.mode not in ('projects', 'conversations', 'unassigned'): return None, ()
        if self.source is w.file_browser.view:
            return ('files', w.shortcuts.paths()) if w.browser_stack.currentWidget() is w.file_browser else (None, ())
        if self.source is w.thread_list:
            if w.detail_tabs.currentWidget() is not w.thread_list: return None, ()
            return 'threads', tuple(i.data(Qt.UserRole) for i in w.thread_list.selectedItems())
        if w.browser_stack.currentWidget() is not w.project_list: return None, ()
        return ('projects', w.selected_project_ids()) if w.mode == 'projects' else ('threads', tuple(i.data(Qt.UserRole) for i in w.project_list.selectedItems()))

    def sync(self):
        kind, items = self.selection(); self.setVisible(bool(items))
        self.w.filter_bar.setVisible(self.w.mode in ('projects', 'conversations', 'unassigned') and self.w.browser_stack.currentWidget() is not self.w.file_browser)
        self.w.filter_bar.setMaximumWidth(175 if items else 16777215)
        if not items: return
        self.summary.setText({'projects': '프로젝트', 'threads': '대화', 'files': '파일·폴더'}[kind] + f' {len(items)}개 선택')
        keys = {'projects': {'move', 'merge', 'backup', 'clear'}, 'threads': {'move', 'rename', 'delete', 'clear'}, 'files': {'copy', 'cut', 'paste', 'delete', 'clear'}}[kind]
        if kind == 'files' and len(items) == 1: keys.add('rename')
        busy = hasattr(self.w, 'updater') and self.w.shortcuts.busy()
        for key, button in self.buttons.items(): button.setVisible(key in keys); button.setEnabled(not busy)
        self.buttons['rename'].setText('제목 변경' if kind == 'threads' else '이름 변경')

    def dispatch(self, key):
        w = self.w
        if w.shortcuts.busy(): return
        kind, items = self.selection()
        if not items: return
        if key == 'clear': self.source.clearSelection(); return
        if kind == 'projects':
            if key == 'move': w.move()
            elif key == 'merge': w.merge()
            elif key == 'backup': w.backup()
        elif kind == 'threads':
            if key == 'move': w.move_selected_threads(items)
            elif key == 'rename': w.rename_selected_thread(items)
            elif key == 'delete': w.delete_selected_threads(items)
        elif kind == 'files':
            if key in ('copy', 'cut'): w.shortcuts.copy(self.source, key == 'cut')
            elif key in ('paste', 'delete', 'rename'): getattr(w.shortcuts, key)(self.source)
