"""Persistent project destinations, independent of the center pane's search."""
from pathlib import Path
from PySide6.QtCore import Qt
from PySide6.QtWidgets import QWidget, QVBoxLayout, QHBoxLayout, QLabel, QLineEdit, QPushButton, QTreeWidget, QTreeWidgetItem, QMenu, QStyle
from ..settings import read_settings, save_settings
from .dragdrop import DragSource


class NavigationTree(DragSource, QTreeWidget):
    pass


class ProjectNavigator(QWidget):
    def __init__(self, window):
        super().__init__(window); self.w = window
        layout = QVBoxLayout(self); layout.setContentsMargins(0, 4, 0, 4); layout.setSpacing(6)
        heading = QHBoxLayout(); title = QLabel('프로젝트'); title.setObjectName('section'); heading.addWidget(title); heading.addStretch()
        self.pin = QPushButton('☆'); self.pin.setObjectName('crumb'); self.pin.setFixedWidth(30)
        self.pin.setToolTip('현재 프로젝트 즐겨찾기 추가 / 해제'); self.pin.clicked.connect(self.toggle_current)
        heading.addWidget(self.pin); layout.addLayout(heading)
        self.search = QLineEdit(); self.search.setPlaceholderText('프로젝트 찾기'); self.search.setClearButtonEnabled(True)
        self.search.textChanged.connect(self.refresh); layout.addWidget(self.search)
        self.tree = NavigationTree(); self.tree.setObjectName('projectTree'); self.tree.setHeaderHidden(True)
        self.tree.setUniformRowHeights(True); self.tree.setTextElideMode(Qt.ElideMiddle); self.tree.setIndentation(14)
        self.tree.setEditTriggers(QTreeWidget.NoEditTriggers); self.tree.setContextMenuPolicy(Qt.CustomContextMenu)
        self.tree.itemClicked.connect(self.activate_item); self.tree.customContextMenuRequested.connect(self.context_menu)
        layout.addWidget(self.tree, 1)
        self.tree.setToolTip('클릭하여 열기 · 프로젝트에 대화나 파일을 끌어 놓기 · 우클릭으로 즐겨찾기')

    def favorites(self):
        saved = read_settings(self.w.state_dir / 'settings.json').get('favorite_projects', {})
        entries = saved.get(str(self.w.adapter.home), []) if isinstance(saved, dict) else []
        return {pid for pid in entries if isinstance(pid, str)} if isinstance(entries, list) else set()

    def toggle_current(self):
        project = self.w.selected_project()
        if project: self.toggle_favorite(project.id)

    def toggle_favorite(self, project_id):
        if not any(p.id == project_id for p in self.w.snapshot.projects): return
        path = self.w.state_dir / 'settings.json'
        saved = read_settings(path).get('favorite_projects', {})
        if not isinstance(saved, dict): saved = {}
        favorites = self.favorites()
        if project_id in favorites: favorites.remove(project_id)
        else: favorites.add(project_id)
        saved[str(self.w.adapter.home)] = sorted(favorites)
        try: save_settings(path, {'favorite_projects': saved})
        except OSError as exc:
            self.w.footer.setText('즐겨찾기를 저장하지 못했습니다: ' + str(exc)); return
        self.refresh()

    def refresh(self):
        expanded = set()
        for i in range(self.tree.topLevelItemCount()):
            group = self.tree.topLevelItem(i)
            for j in range(group.childCount()):
                item = group.child(j)
                if item.isExpanded(): expanded.add(item.data(0, Qt.UserRole)['project_id'])
        query = self.search.text().strip().casefold(); favorites = self.favorites()
        projects = sorted(self.w.snapshot.projects, key=lambda p: p.name.casefold())
        projects = [p for p in projects if not query or query in (p.name + ' ' + ' '.join(map(str, p.roots))).casefold()]
        self.tree.clear()
        for title, group_projects in [('즐겨찾기', [p for p in projects if p.id in favorites]), ('모든 프로젝트', [p for p in projects if p.id not in favorites])]:
            if not group_projects: continue
            group = QTreeWidgetItem([f'{title} · {len(group_projects)}']); group.setFlags(Qt.ItemIsEnabled)
            self.tree.addTopLevelItem(group); group.setExpanded(True)
            for project in group_projects:
                item = QTreeWidgetItem([project.name]); item.setData(0, Qt.UserRole, {'project_id': project.id})
                item.setIcon(0, self.style().standardIcon(QStyle.SP_DirIcon)); item.setToolTip(0, project.name + '\n' + '\n'.join(map(str, project.roots)))
                group.addChild(item)
                for root in project.roots:
                    child = QTreeWidgetItem([root.name or str(root)]); child.setData(0, Qt.UserRole, {'project_id': project.id, 'root': str(root)})
                    child.setFlags(child.flags() & ~Qt.ItemIsDragEnabled); child.setToolTip(0, str(root)); item.addChild(child)
                item.setExpanded(project.id in expanded)
        self.sync_current()

    def project_item(self, project_id):
        for i in range(self.tree.topLevelItemCount()):
            group = self.tree.topLevelItem(i)
            for j in range(group.childCount()):
                item = group.child(j)
                if item.data(0, Qt.UserRole)['project_id'] == project_id: return item
        return None

    def sync_current(self):
        project = self.w.selected_project() if hasattr(self.w, 'project_list') else None
        self.pin.setEnabled(project is not None); self.pin.setText('★' if project and project.id in self.favorites() else '☆')
        item = self.project_item(project.id) if project else None
        self.tree.setCurrentItem(item)

    def activate_item(self, item, _column=0):
        data = item.data(0, Qt.UserRole)
        if data: self.activate_project(data['project_id'], data.get('root'))

    def activate_project(self, project_id, root=None):
        w = self.w
        if not any(p.id == project_id for p in w.snapshot.projects): return
        if w.mode != 'projects': w.set_mode('projects')
        w.search.clear(); w.show_projects(); w.project_list.clearSelection()
        for i in range(w.project_list.count()):
            item = w.project_list.item(i)
            if item.data(Qt.UserRole) == project_id:
                w.project_list.setCurrentItem(item); item.setSelected(True); w.open_project()
                if root: w.file_browser.navigate(Path(root))
                break

    def context_menu(self, point):
        item = self.tree.itemAt(point); data = item.data(0, Qt.UserRole) if item else None
        if not data: return
        menu = QMenu(self); pid = data['project_id']
        menu.addAction('열기', lambda: self.activate_item(item))
        menu.addAction('즐겨찾기 해제' if pid in self.favorites() else '즐겨찾기 추가', lambda: self.toggle_favorite(pid))
        menu.exec(self.tree.viewport().mapToGlobal(point))
