import json
import re
import threading
from datetime import datetime,timezone,timedelta
from pathlib import Path
from PySide6.QtCore import Qt,QThread,Signal,QUrl,QTimer
from PySide6.QtGui import QDesktopServices,QIcon
from PySide6.QtWidgets import (QMainWindow,QWidget,QHBoxLayout,QVBoxLayout,QLabel,QPushButton,
    QLineEdit,QListWidget,QListWidgetItem,QSplitter,QFileDialog,QMessageBox,QProgressDialog,
    QDialog,QPlainTextEdit,QInputDialog,QAbstractItemView,QFrame,QComboBox,QMenu,QApplication,QStyle)
from ..models import Snapshot
from ..catalog import clean_path
from ..codex.adapter import CodexAdapter
from ..codex.portability import transcript
from ..bundles import export_project,verify_bundle,load_manifest,read_transcript
from ..files import scan_roots
from ..journal import Journal
from ..operations import transfer_project,import_bundle,recover_operation,change_connections
from ..restore_proof import verify_restored_bundle
from .dialogs import confirm,confirm_transfer,ConnectionsDialog
from .updates import UpdateChecker,UpdateDialog,UpdateManager
from .sizes import SizeScanner
from ..grouping import display_membership
from ..stores import Store,store_info,discover_stores,default_search_roots
from ..version import APP_NAME,VERSION
from ..settings import read_settings,save_settings
from .management import ManagementActions


def bytes_text(size):
    if size is None: return '용량 미계산'
    for unit in ('B','KB','MB','GB','TB'):
        if size<1024 or unit=='TB': return f'{size:.1f} {unit}'
        size/=1024


class Worker(QThread):
    result=Signal(object);error=Signal(str);progress=Signal(object,object)
    def __init__(self,func):
        super().__init__();self.func=func;self.cancelled=threading.Event()
    def run(self):
        try: self.result.emit(self.func(self))
        except Exception as exc: self.error.emit(str(exc))


class MainWindow(ManagementActions,QMainWindow):
    def __init__(self,adapter,state_dir,auto_refresh=True,start_update_check=True):
        super().__init__();self.adapter=adapter;self.state_dir=Path(state_dir)
        from .appearance import apply_light_theme
        apply_light_theme(QApplication.instance())
        self.setWindowIcon(QIcon(str(Path(__file__).parent/'app-icon.ico')))
        self.journal=Journal(self.state_dir/'journal.sqlite');self.snapshot=Snapshot((),(),'')
        self.mode='projects';self.section_queries={};self.workers=[];self.sizes={};self.backups=[];self.active_bundle=None
        self.size_scanner=SizeScanner(self);self._closing=False
        self._store_worker=None;self.stores={};self.store_dialog=None
        saved_stores=read_settings(self.state_dir/'settings.json').get('homes',[])
        for home in saved_stores if isinstance(saved_stores,list) else []:
            if isinstance(home,str) and home:
                try:self.stores[Path(home).resolve()]=Store(Path(home).resolve(),False,error='확인 전')
                except (ValueError,OSError):pass
        self.stores[adapter.home]=Store(adapter.home,True)
        self.membership=display_membership(self.snapshot,{})
        self.startup_update_check=True;self.latest_release=None;self.update_dialog=None;self.update_popup_pending=False
        self.activity_dialog=None
        settings_path=self.state_dir/'settings.json'
        if settings_path.exists():
            try:
                saved=read_settings(settings_path)
                backups=saved.get('backups',[])
                self.backups=[Path(p) for p in backups if isinstance(p,str) and p] if isinstance(backups,list) else []
                self.startup_update_check=saved.get('startup_update_check',True) is not False
            except (ValueError,OSError): pass
        self.backups.extend(p for p in self.journal.verified_backups(adapter.home) if p not in self.backups)
        self.setWindowTitle(APP_NAME);self.resize(1260,810);self.setMinimumSize(950,630)
        from .layout import build_layout
        self.browsing_project=None
        build_layout(self)
        self.set_mode('projects')
        self.updater=UpdateManager(self);self.updater.handoff.connect(self.close)
        self.updater.changed.connect(self.sync_size_scan)
        self.size_scanner.changed.connect(self.size_measured);self.size_scanner.settled.connect(self.sizes_settled)
        self.update_checker=UpdateChecker(self);self.update_checker.checked.connect(self.update_checked);self.update_checker.failed.connect(self.update_failed)
        self.refresh_store_picker()
        if auto_refresh: QTimer.singleShot(0,self.reload)
        if auto_refresh: QTimer.singleShot(1500,self.start_store_scan)
        if auto_refresh and start_update_check and self.startup_update_check: QTimer.singleShot(1000,lambda:self.check_updates(True))

    def open_updates(self,check=True):
        if not self.update_dialog: self.update_dialog=UpdateDialog(self)
        self.update_dialog.show();self.update_dialog.raise_()
        if self.latest_release: self.update_dialog.show_release(self.latest_release)
        if check: self.check_updates(False)
        return self.update_dialog

    def check_updates(self,startup=False):
        if self.updater.requested: return
        self.update_popup_pending=self.update_popup_pending or startup
        if self.update_dialog: self.update_dialog.status.setText('업데이트 확인 중…')
        self.update_checker.check()

    def update_checked(self,release):
        self.latest_release=release
        self.updater.prepare(release)
        if self.update_dialog: self.update_dialog.show_release(release)
        if self.update_popup_pending and release and release.available:
            if self.workers: QTimer.singleShot(1000,lambda:self.update_checked(release));return
            self.open_updates(check=False)
        self.update_popup_pending=False

    def update_failed(self,error):
        self.update_popup_pending=False
        if self.update_dialog: self.update_dialog.status.setText('확인하지 못했습니다. 다시 시도할 수 있습니다.\n'+error)
        self.footer.setText('업데이트 확인 실패 · 업데이트 메뉴에서 다시 확인할 수 있습니다.')

    def selected_project(self):
        item=self.project_list.currentItem()
        if not item or self.mode!='projects': return None
        return next((p for p in self.snapshot.projects if p.id==item.data(Qt.UserRole)),None)

    def selected_project_ids(self):
        return tuple(p.id for p in self.selected_projects())

    def selected_projects(self):
        if self.mode!='projects':return ()
        ids={item.data(Qt.UserRole) for item in self.project_list.selectedItems()}
        return tuple(p for p in self.snapshot.projects if p.id in ids)

    def set_mode(self,mode):
        self.project_list.blockSignals(True)
        self.section_queries[self.mode]=self.search.text()
        self.search.blockSignals(True);self.search.setText(self.section_queries.get(mode,''));self.search.blockSignals(False)
        self.browsing_project=None;self.browser_stack.setCurrentWidget(self.project_list)
        self.mode=mode
        self.show_internal.setVisible(mode in ('projects','unassigned','conversations'))
        self.project_list.setSelectionMode(QAbstractItemView.ExtendedSelection if mode in ('projects','unassigned','conversations') else QAbstractItemView.SingleSelection)
        for b,m in self.nav_buttons: b.setChecked(m==mode)
        self.heading.setText({'projects':'모든 프로젝트','conversations':'모든 대화','unassigned':'분류할 대화','backups':'백업','recovery':'작업 복구'}[mode])
        self.chat_status.setVisible(mode in ('projects','unassigned','conversations'));self.thread_search.setVisible(mode=='projects')
        self.import_button.setVisible(mode in ('projects','backups'));self.size_status.setVisible(mode=='projects')
        self.project_actions.setVisible(mode=='projects');self.home_button.hide()
        self.subtitle.setText({'projects':'Ctrl·Shift로 여러 개 선택 → 왼쪽 프로젝트에 끌어 놓아 합치기 · 열 제목 클릭으로 정렬','conversations':'여러 대화를 선택한 뒤 왼쪽 대상 프로젝트에 끌어 놓으면 함께 옮길 수 있습니다.','unassigned':'프로젝트 연결이 없는 대화입니다. 보조 에이전트·자동 검토 기록은 내부 작업 기록 포함에서 확인하세요.','backups':'파일과 대화가 함께 담긴 백업입니다. 백업을 선택하고 복원할 위치를 지정하세요.','recovery':'삭제한 항목을 되살리거나 중단된 작업을 복구합니다. 완료·실패 결과는 작업 내역에서 확인하세요.'}[mode])
        self.project_list.clear();self.project_list.blockSignals(False)
        self.footer.setText('Ctrl·Shift로 여러 개 선택 · Ctrl+A 전체 선택 · F2 이름 변경 · Delete 삭제' if mode in ('projects','conversations','unassigned') else '항목을 선택하면 오른쪽에서 상세 작업을 확인할 수 있습니다.')
        self.filter_list()
        self.selection_bar.activate(self.project_list)

    def show_projects(self):
        if self.mode!='projects':self.set_mode('projects');return
        self.browsing_project=None;self.browser_stack.setCurrentWidget(self.project_list);self.home_button.hide();self.heading.setText('모든 프로젝트');self.update_empty_state()
        self.selection_bar.activate(self.project_list)

    def navigate_back(self):
        if self.browser_stack.currentWidget()==self.file_browser:self.file_browser.go_back()
        elif self.mode!='projects':self.set_mode('projects')

    def open_project(self):
        p=self.selected_project()
        if not p:return
        self.browsing_project=p.id;self.file_browser.set_project(p);self.browser_stack.setCurrentWidget(self.file_browser)
        self.home_button.show();self.heading.setText(p.name);self.empty_state.hide()
        self.selection_bar.activate(self.file_browser.view)

    def open_selected(self,item,*_):
        if self.mode=='projects':self.open_project()
        elif self.mode in ('unassigned','conversations'):self.show_transcript(item)
        elif self.mode=='backups':self.restore()
        elif self.mode=='recovery':self.show_recovery()

    def open_root(self,item):
        p=self.selected_project()
        if p:
            self.open_project();self.file_browser.navigate(Path(item.data(Qt.UserRole)))

    def project_context_menu(self,point):
        item=self.project_list.itemAt(point)
        if not item:return
        if not item.isSelected():self.project_list.clearSelection();self.project_list.setCurrentItem(item)
        menu=QMenu(self)
        if self.mode=='projects':
            for title,callback in [('열기',self.open_project),('이름 변경…',self.rename_selected_project),('옮기기…',self.move),('다른 프로젝트와 합치기…',self.merge),('백업 만들기…',self.backup),('프로젝트 설정…',self.connections),('프로젝트 삭제…',self.delete_selected_project)]:
                action=menu.addAction(title,callback);action.setEnabled(len(self.selected_projects())<=1 or callback in (self.move,self.merge,self.backup))
            project=self.selected_project()
            if project and project.roots:menu.addAction('폴더 경로 복사',lambda:QApplication.clipboard().setText(str(project.roots[0])))
        elif self.mode=='backups':menu.addAction('이 백업 복원하기…',self.restore)
        elif self.mode in ('unassigned','conversations'):
            preview=QListWidgetItem(item.text());preview.setData(Qt.UserRole,item.data(Qt.UserRole))
            menu.addAction('대화 읽기',lambda:self.show_transcript(preview))
            ids=tuple(i.data(Qt.UserRole) for i in self.project_list.selectedItems())
            menu.addAction(f'프로젝트로 이동… ({len(ids)}개)',lambda:self.move_selected_threads(ids))
            menu.addAction('제목 변경…' if len(ids)==1 else '제목 일괄 변경…',lambda:self.rename_selected_thread(ids))
            menu.addAction('제목 복사',lambda:self.copy_thread_titles(ids))
            menu.addAction('폴더 경로 복사',lambda:QApplication.clipboard().setText(str(next(t.cwd for t in self.snapshot.conversations if t.id==preview.data(Qt.UserRole)))))
            menu.addAction(f'대화 삭제… ({len(ids)}개)',lambda:self.delete_selected_threads(ids))
        else:menu.addAction('중단된 작업 복구…',self.show_recovery)
        self.shortcuts.add_menu(menu,self.project_list)
        menu.exec(self.project_list.viewport().mapToGlobal(point))

    def show_help(self):
        QMessageBox.information(self,'Codex Manager 사용 안내','1. 프로젝트를 두 번 클릭하면 파일과 폴더가 열립니다.\n2. 오른쪽 대화를 두 번 클릭하면 내용을 읽을 수 있습니다.\n3. 프로젝트를 선택하고 옮기기·합치기·백업 만들기를 누르세요.\n4. 프로젝트 우클릭으로 이름 변경·삭제를 할 수 있습니다.\n5. 대화는 Ctrl·Shift로 여러 개 선택하고 대화 관리에서 제목 변경·이동·삭제합니다.\n6. 삭제한 항목은 작업 복구에서 되살릴 수 있습니다.\n\n외장하드에 보관하기\n백업 만들기 → 외장하드 선택. 파일과 대화를 함께 보관합니다.\n\n다른 컴퓨터로 가져오기\n백업 가져오기 → 백업 폴더 선택 → 이 백업 복원하기.\n\n원본 정리\n복원한 대화에서 실제로 이어 쓴 뒤 백업의 복원 상태를 확인하세요. 확인을 마치면 원래 프로젝트의 더 보기에서 원본을 정리할 수 있습니다.\n\n이동·합치기·복원은 Codex 앱을 종료한 상태에서 진행하세요.\nCtrl+F 검색 · F5 새로고침 · Alt+← 뒤로')

    def update_empty_state(self):
        empty=self.project_list.count()==0 and self.browser_stack.currentWidget()==self.project_list
        self.empty_state.setVisible(empty)
        if self.search.text():text='검색 결과가 없습니다. 다른 이름이나 폴더 경로를 검색해 보세요.'
        else:text={'projects':'아직 프로젝트가 없습니다.\n다른 위치 찾기로 기존 Codex 데이터를 연결하거나 백업을 가져오세요.','conversations':'표시할 대화가 없습니다. 필터를 변경하거나 다른 저장소를 확인하세요.','unassigned':'분류할 대화가 없습니다.','backups':'아직 등록된 백업이 없습니다.\n프로젝트에서 백업 만들기를 누르거나 기존 백업을 가져오세요.','recovery':'삭제한 항목이나 복구할 작업이 없습니다.'}[self.mode]
        self.empty_state.setText(text)

    def refresh(self,snapshot):
        self.snapshot=snapshot
        self.backups.extend(p for p in self.journal.verified_backups(self.adapter.home) if p not in self.backups)
        self.membership=display_membership(snapshot,read_settings(self.adapter.home/'.codex-global-state.json'))
        self.size_scanner.reset(self.ordered_projects());self.sizes={pid:r.total for pid,r in self.size_scanner.results.items()};self.filter_list();self.update_size_status()
        self.navigator.refresh()
        if self.activity_dialog:self.activity_dialog.refresh()

    def reload(self):
        self.run_job('프로젝트 확인',lambda w:self.adapter.snapshot(include_runtime=False),self.refresh)

    def filter_list(self):
        query=self.search.text().lower();selected=self.project_list.currentItem()
        old=selected.data(Qt.UserRole) if selected else None
        old_selection={i.data(Qt.UserRole) for i in self.project_list.selectedItems()}
        scroll=self.project_list.verticalScrollBar().value();self.project_list.blockSignals(True);self.project_list.clear()
        if self.mode=='projects':
            self.project_list.configure(['이름','폴더 위치','대화','용량'])
            projects=self.ordered_projects();visible_ids={t.id for t in self.visible_conversations()}
            for p in projects:
                ts=self.project_threads(p.id)
                if query and query not in p.name.lower() and not any(query in str(r).lower() for r in p.roots) and not any(query in t.title.lower() for t in ts): continue
                item=self.project_list.add_record([p.name,str(p.roots[0])+(f' 외 {len(p.roots)-1}곳' if len(p.roots)>1 else '') if p.roots else '폴더 없음',str(sum(t.id in visible_ids for t in ts)),self.size_text(p.id)],p.id,self.style().standardIcon(QStyle.SP_DirIcon));item.setToolTip(3,self.size_tooltip(p.id))
                if p.id==old: self.project_list.setCurrentItem(item)
        elif self.mode in ('unassigned','conversations'):
            self.project_list.configure(['대화 제목','프로젝트','최근 작업','상태'])
            # Column widths remain user adjustable across refreshes.
            project_names={p.id:p.name for p in self.snapshot.projects}
            threads=self.visible_conversations(query=query)
            for t in threads:
                pid=self.membership.projects.get(t.id)
                if self.mode=='unassigned' and pid is not None:continue
                state=('보관됨' if t.archived else '일반 대화')+(' · 내부' if t.internal else '')
                item=self.project_list.add_record([t.title,project_names.get(pid,'프로젝트 없음'),self.chat_date(t.updated_at),state],t.id)
                item.setToolTip(0,t.title+'\n'+str(t.cwd))
        elif self.mode=='backups':
            self.project_list.configure(['프로젝트','백업 위치'])
            for bundle in self.backups:
                try: m=load_manifest(bundle);name=m['project']['name']
                except Exception: name='연결되지 않은 백업' if not bundle.exists() else '확인 필요한 백업'
                if query and query not in (name+' '+str(bundle)).lower():continue
                item=QListWidgetItem(name+'\n'+str(bundle));item.setData(Qt.UserRole,str(bundle));added=self.project_list.addItem(item)
                if str(bundle)==old:self.project_list.setCurrentItem(added)
        else:
            self.project_list.configure(['작업','작업 번호'])
            for operation in self.recovery_items():
                if query and query not in (operation['payload'].get('kind','작업')+' '+operation['id']).lower():continue
                title=(operation['payload'].get('label','파일 작업')+' · 되돌리기') if operation['payload'].get('kind')=='file-edit' else '프로젝트·대화 연결 되돌리기' if operation['payload'].get('kind')=='connections' and operation['state']=='completed' else ('제목 변경 되돌리기 · '+operation['payload'].get('label',operation['payload'].get('new_name',''))) if operation['payload'].get('kind') in ('rename-thread','rename-threads') else ('삭제 복구 · '+operation['payload'].get('label','')) if operation['payload'].get('kind')=='management-delete' else operation['payload'].get('kind','작업')+' · '+operation['state']
                item=QListWidgetItem(title+'\n'+operation['id']);item.setData(Qt.UserRole,operation['id']);self.project_list.addItem(item)
        if old is not None and self.project_list.currentItem() is None:
            for n in range(self.project_list.count()):
                if self.project_list.item(n).data(Qt.UserRole)==old:self.project_list.setCurrentRow(n);break
        if self.mode in ('projects','unassigned','conversations'):
            for n in range(self.project_list.count()):
                item=self.project_list.item(n);item.setSelected(item.data(Qt.UserRole) in old_selection)
        self.column_sort.indicator();self.project_list.blockSignals(False);self.project_list.verticalScrollBar().setValue(scroll);self.show_detail();self.update_empty_state()

    def show_detail(self,*_):
        selected_threads={i.data(Qt.UserRole) for i in self.thread_list.selectedItems()}
        self.folder_list.clear();self.thread_list.clear();self.detail_status.setText('');self.folder_size.setText('');self.confirm_links.hide();self.active_bundle=None
        self.bundle_action.setVisible(self.mode=='backups');self.proof_action.setVisible(self.mode=='backups');self.recovery_action.setVisible(self.mode=='recovery')
        self.assign_action.setVisible(self.mode=='unassigned')
        p=self.selected_project()
        self.open_button.setEnabled(p is not None);self.more.setEnabled(p is not None)
        self.bundle_action.setEnabled(False);self.proof_action.setEnabled(False);self.assign_action.setEnabled(bool(self.project_list.selectedItems()));self.recovery_action.setEnabled(self.project_list.currentItem() is not None)
        if self.browsing_project and (not p or p.id!=self.browsing_project):self.show_projects()
        if self.browsing_project and p:self.file_browser.set_project(p);self.heading.setText(p.name)
        for key,b in self.buttons.items():b.setEnabled(p is not None and (key in ('move','merge','backup') or len(self.selected_projects())<=1))
        self.cleanup_button.setEnabled(False);self.cleanup_button.setToolTip('실제 계정의 복원 후 이어쓰기 검증 전에는 원본 정리를 사용하지 않습니다.')
        if p:
            for bundle in self.backups:
                try:
                    m=load_manifest(bundle);proof=json.loads((bundle/'verification.json').read_text(encoding='utf-8'))
                    if m['project']['id']==p.id and proof.get('restore_verified') and len(self.selected_projects())==1:self.cleanup_button.setEnabled(True)
                except (ValueError,OSError,TypeError,AttributeError): pass
            self.detail_title.setText(p.name)
            for path in p.roots:
                item=QListWidgetItem(str(path)+('' if path.exists() else '\n폴더 없음'));item.setToolTip(str(path));item.setData(Qt.UserRole,str(path));self.folder_list.addItem(item)
            for t in self.visible_conversations(query=self.thread_search.text(),project_only=True):
                if self.membership.projects.get(t.id)==p.id:
                    label=t.title+('  ·  내부 작업' if t.internal else '')+('  ·  보관됨' if t.archived else '')+('  ·  원래 대화 기준' if self.membership.reasons.get(t.id)=='parent' else '  ·  폴더 기준' if t.id in self.membership.inferred else '')
                    item=QListWidgetItem(label);item.setToolTip(str(t.cwd));item.setData(Qt.UserRole,t.id);self.thread_list.addItem(item)
            self.detail_status.setText('대화를 두 번 클릭하면 내용을 읽을 수 있습니다.' if self.thread_list.count() else '이 프로젝트에 표시할 대화가 없습니다.')
            self.folder_size.setText(self.size_text(p.id));self.folder_size.setToolTip(self.size_tooltip(p.id))
            inferred=self.folder_threads(p.id)
            if inferred:
                self.confirm_links.show();self.confirm_links.setText(f'대화 {len(inferred)}개 연결 확인')
                self.detail_status.setText('폴더 또는 원래 대화를 기준으로 함께 표시합니다. 이동·합치기 전 필요한 연결을 안내합니다.')
        else:
            self.detail_title.setText('프로젝트를 선택하세요' if self.mode=='projects' else '항목을 선택하세요')
            self.detail_status.setText('선택한 항목의 대화와 폴더가 여기에 표시됩니다.')
            item=self.project_list.currentItem() or next(iter(self.project_list.selectedItems()),None)
            if item and self.mode in ('unassigned','conversations'):
                tid=item.data(Qt.UserRole);t=next(t for t in self.snapshot.conversations if t.id==tid)
                ids={i.data(Qt.UserRole) for i in self.project_list.selectedItems()} or {tid}
                self.detail_title.setText(t.title if len(ids)==1 else f'대화 {len(ids)}개 선택')
                for selected_thread in self.snapshot.conversations:
                    if selected_thread.id in ids:
                        i=QListWidgetItem(selected_thread.title);i.setData(Qt.UserRole,selected_thread.id);self.thread_list.addItem(i)
                reason=self.membership.reasons.get(tid,'outside')
                self.detail_status.setText(('프로젝트: '+next((p.name for p in self.snapshot.projects if p.id==self.membership.projects.get(tid)),'없음')+'\n'+str(t.cwd)) if self.mode=='conversations' else {'internal_parent':'원래 대화의 프로젝트 연결을 확인할 수 없는 내부 작업 기록입니다.','ambiguous':'여러 프로젝트가 같은 폴더를 사용합니다. 연결할 프로젝트를 선택하세요.','projectless':'Codex에서 프로젝트 없는 대화로 지정됐습니다.','missing_project':'기록된 프로젝트를 현재 저장소에서 찾을 수 없습니다.','other_host':'다른 연결 호스트의 대화입니다.'}.get(reason,'현재 프로젝트 폴더와 맞는 경로가 없습니다. 옛 경로이거나 프로젝트가 제거됐을 수 있습니다.'))
            if item and self.mode=='backups':
                self.active_bundle=Path(item.data(Qt.UserRole))
                try:
                    m=load_manifest(self.active_bundle);self.detail_title.setText(m['project']['name'])
                    self.bundle_action.setEnabled(True);self.proof_action.setEnabled(True)
                    for r in m['roots']: self.folder_list.addItem(r['original_path'])
                    for t in m['conversations']:
                        i=QListWidgetItem(t['title']);i.setData(Qt.UserRole,t['id']);self.thread_list.addItem(i)
                    self.detail_status.setText('① 이 백업 복원하기 → 새 폴더 선택\n② Codex에서 복원한 대화 이어쓰기\n③ 복원 상태 확인 → 원래 컴퓨터에서 원본 정리')
                except Exception as exc:
                    self.detail_status.setText('백업을 연결하거나 파일 상태를 확인하세요.\n'+str(exc))
                    self.bundle_action.setEnabled(False);self.proof_action.setEnabled(False)
        self.detail_tabs.setTabText(0,f'대화 {self.thread_list.count()}');self.detail_tabs.setTabText(1,f'폴더 위치 {self.folder_list.count()}')
        for n in range(self.thread_list.count()):
            item=self.thread_list.item(n)
            if item.data(Qt.UserRole) in selected_threads:item.setSelected(True)
        self.detail_title.setToolTip(self.detail_title.text());self.detail_status.setToolTip(self.detail_status.text())
        self.sync_thread_actions()
        self.navigator.sync_current()

    def run_job(self,title,func,done=None):
        if self.updater.requested:
            self.footer.setText('업데이트 설치가 진행 중입니다. 완료 후 작업할 수 있습니다.');return
        if self.workers: return
        self.size_scanner.stop()
        dialog=QProgressDialog(title,'취소',0,0,self);dialog.setWindowTitle('작업 중');dialog.setWindowModality(Qt.WindowModal);dialog.setMinimumDuration(0);dialog.setAutoClose(False);dialog.setAutoReset(False)
        worker=Worker(func);self.workers.append(worker);dialog.canceled.connect(worker.cancelled.set)
        self.selection_bar.sync()
        def progress(current,total):
            if total: dialog.setRange(0,1000);dialog.setValue(int(current/total*1000));dialog.setLabelText(f'{title}\n{bytes_text(current)} / {bytes_text(total)}')
        worker.progress.connect(progress)
        def success(result):
            dialog.close()
            if done: done(result)
        worker.result.connect(success);worker.error.connect(lambda e:(dialog.close(),QMessageBox.warning(self,'작업 확인',e)))
        def finish():
            self.workers.remove(worker);worker.deleteLater();self.sync_size_scan()
            self.selection_bar.sync()
            if self.activity_dialog:self.activity_dialog.refresh()
        worker.finished.connect(finish);worker.start()

    def show_result(self,result):
        if self.updater.requested:
            self.footer.setText('프로젝트 작업 완료 · 업데이트를 계속합니다.');return
        if hasattr(result,'state'):
            if result.report_id:self.open_activity().show_result(result)
            elif result.errors:QMessageBox.warning(self,'작업 결과','\n'.join([result.file_status,*result.errors]))
            self.footer.setText(result.file_status+' · 작업 내역에서 결과를 확인할 수 있습니다.')
        else: QMessageBox.information(self,'작업 결과','검증을 마쳤습니다.')
        QTimer.singleShot(50,self.reload)

    def open_activity(self):
        from .activity import ActivityDialog
        if not self.activity_dialog:self.activity_dialog=ActivityDialog(self)
        self.activity_dialog.refresh();self.activity_dialog.show();self.activity_dialog.raise_()
        return self.activity_dialog

    def retry_activity(self,operation_id):
        from .activity import retry_candidates
        if self.shortcuts.busy():return
        operation=self.journal.operation(operation_id)
        if not operation or Path(operation['payload'].get('home',''))!=self.adapter.home:return
        ids=retry_candidates(operation,self.journal)
        if not ids:return
        def ready(snapshot):
            self.refresh(snapshot)
            def start():
                if self.workers:QTimer.singleShot(20,start);return
                current=self.journal.operation(operation_id)
                if not current or retry_candidates(current,self.journal)!=ids:return
                projects=tuple(p for p in snapshot.projects if p.id in ids)
                if len(projects)!=len(ids):
                    QMessageBox.information(self,'다시 시도','프로젝트가 삭제되거나 다른 작업으로 합쳐졌습니다. 현재 프로젝트 목록에서 대상을 다시 선택하세요.');return
                payload=operation['payload']
                if payload['action']=='backup':self.backup(projects=projects,retry_of=operation_id);return
                target=next((p for p in snapshot.projects if p.id==payload.get('target_id')),None)
                if payload['action']=='merge' and not target:
                    QMessageBox.information(self,'다시 시도','대상 프로젝트를 찾을 수 없습니다. 현재 목록에서 합칠 대상을 다시 선택하세요.');return
                parent=None
                if not target:
                    folder=QFileDialog.getExistingDirectory(self,'미완료 프로젝트를 옮길 폴더')
                    if not folder:return
                    parent=Path(folder)
                self.transfer_projects(projects,target=target,parent=parent,retry_of=operation_id)
            QTimer.singleShot(0,start)
        self.run_job('재시도할 프로젝트 확인',lambda worker:self.adapter.snapshot(include_runtime=False),ready)

    def ordered_projects(self):
        return self.column_sort.projects(self.snapshot.projects)

    @staticmethod
    def chat_date(value):
        try:return datetime.fromtimestamp(value).strftime('%Y-%m-%d %H:%M') if value else '—'
        except (ValueError,OSError,OverflowError):return '—'

    def visible_conversations(self,query='',project_only=False):
        names={p.id:p.name for p in self.snapshot.projects};query=query.casefold().strip()
        status=self.chat_status.currentIndex();result=[]
        for t in self.snapshot.conversations:
            if t.internal and not self.show_internal.isChecked():continue
            if status==1 and t.archived or status==2 and not t.archived:continue
            text=t.title if project_only else t.title+' '+str(t.cwd)+' '+names.get(self.membership.projects.get(t.id),'')
            if query and query not in text.casefold():continue
            result.append(t)
        return self.column_sort.chats(result,names,project_only)

    def project_threads(self,pid):return [t for t in self.snapshot.conversations if self.membership.projects.get(t.id)==pid]
    def folder_threads(self,pid):return [t for t in self.snapshot.conversations if self.membership.inferred.get(t.id)==pid]
    def size_text(self,pid):
        result=self.size_scanner.results.get(pid)
        if result is None:return '자동 계산 중…'
        if result.skipped and not result.files:return '확인 필요'
        return bytes_text(result.total)+(' 이상 · 일부 제외' if result.skipped else '')+(' · 갱신 중' if pid in self.size_scanner.pending else '')
    def size_tooltip(self,pid):
        result=self.size_scanner.results.get(pid)
        if not result:return '파일을 읽거나 해시를 계산하지 않고 폴더 크기를 확인합니다.'
        return f'파일 크기 합계 · {result.files:,}개 파일\n연결 폴더·클라우드 자리표시자는 제외합니다.'+('\n'+'\n'.join(result.warnings) if result.warnings else '')
    def project_label(self,p):return f'{p.name}\n{len(self.project_threads(p.id))}개 대화  ·  {len(p.roots)}개 폴더  ·  {self.size_text(p.id)}'
    def update_size_status(self):
        total=len(self.snapshot.projects);remaining=len(self.size_scanner.pending)
        self.size_status.setText(f'용량 자동 계산 {total-remaining}/{total}'+(' · 계산 중에도 작업할 수 있습니다.' if remaining else ' · 파일 크기 합계'))
    def size_measured(self,pid):
        self.sizes[pid]=self.size_scanner.results[pid].total
        p=next((p for p in self.snapshot.projects if p.id==pid),None)
        if p and self.mode=='projects':
            for n in range(self.project_list.count()):
                item=self.project_list.item(n)
                if item.data(Qt.UserRole)==pid:item.setText(3,self.size_text(pid));item.setToolTip(3,self.size_tooltip(pid));break
        selected=self.selected_project()
        if selected and selected.id==pid:self.folder_size.setText(self.size_text(pid));self.folder_size.setToolTip(self.size_tooltip(pid))
        self.update_size_status()
    def sizes_settled(self):
        if not self._closing:self.filter_list();self.update_size_status()
    def sync_size_scan(self):
        busy=self.updater.requested or self.updater._close_requested or self._closing
        self.centralWidget().setEnabled(not busy)
        if busy or self.workers:self.size_scanner.stop()
        else:self.size_scanner.resume()

    def stop_background(self):
        idle=self.size_scanner.stop()
        if self._store_worker:self._store_worker.cancelled.set();idle=False
        return idle

    def refresh_store_picker(self):
        self.store_picker.blockSignals(True);self.store_picker.clear()
        for home,info in self.stores.items():
            label=home.name+' · '+home.parent.name+(' · 연결 안 됨' if not info.available else '')
            self.store_picker.addItem(label,str(home));self.store_picker.setItemData(self.store_picker.count()-1,str(home),Qt.ToolTipRole)
            if home==self.adapter.home:self.store_picker.setCurrentIndex(self.store_picker.count()-1)
        self.store_picker.blockSignals(False);self.connection.setText(self.adapter.home.parent.name+' / '+self.adapter.home.name);self.connection.setToolTip(str(self.adapter.home))
        if self.store_dialog:self.store_dialog.refresh()

    def start_store_scan(self,extra=None):
        if self._closing or self.updater.requested:return
        if self.workers:
            QTimer.singleShot(500,lambda:self.start_store_scan(extra));return
        if self._store_worker:return
        import os
        known=list(self.stores)
        for home in (Path.home()/'.codex',Path(os.environ.get('CODEX_HOME',str(self.adapter.home)))):
            if home not in known:known.append(home)
        roots=(Path(extra),) if extra else default_search_roots(known)
        worker=Worker(lambda w:discover_stores(known,roots,w.cancelled.is_set));self._store_worker=worker
        if self.store_dialog:self.store_dialog.status.setText('저장소를 자동으로 찾는 중…')
        def found(result):
            if worker.cancelled.is_set():return
            self.stores.update({s.home:s for s in result.stores});self.refresh_store_picker();self.save_settings()
            text=f'저장소 {len(result.stores)}개 확인 · 폴더 {result.visited:,}개 검색'+(' · 검색 범위 한도에 도달했습니다. 추가 검색 위치를 선택하세요.' if result.limited else '')
            if self.store_dialog:self.store_dialog.status.setText(text)
            else:self.footer.setText(text)
        worker.result.connect(found)
        worker.error.connect(lambda error:self.footer.setText('저장소 검색: '+error) if not worker.cancelled.is_set() else None)
        def finish():self._store_worker=None;worker.deleteLater();self.sync_size_scan()
        worker.finished.connect(finish);worker.start()

    def open_stores(self):
        from .stores import StoresDialog
        if not self.store_dialog:self.store_dialog=StoresDialog(self)
        self.store_dialog.refresh();self.store_dialog.show();self.store_dialog.raise_();self.start_store_scan()

    def switch_store(self,index):
        if index<0:return
        home=Path(self.store_picker.itemData(index))
        if home==self.adapter.home:return
        self.connect_store(home)

    def connect_store(self,home):
        if self.workers or self.updater.requested:self.refresh_store_picker();return
        self.refresh_store_picker();isolated=self.adapter.isolated
        def verify(worker):
            info=store_info(home,worker.cancelled.is_set)
            if not info.available:raise ValueError(str(home)+'\n'+info.error)
            adapter=CodexAdapter(home,isolated=isolated);snapshot=adapter.snapshot(include_runtime=False)
            if worker.cancelled.is_set():raise InterruptedError('저장소 전환을 취소했습니다.')
            return info,adapter,snapshot
        def connected(value):
            info,adapter,snapshot=value
            self.size_scanner.stop();self.size_scanner.results.clear();self.sizes.clear()
            self.stores[info.home]=info;self.adapter=adapter;self.refresh(snapshot);self.refresh_store_picker();self.save_settings()
        self.run_job('저장소 연결 확인',verify,connected)

    def connect_folder_threads(self):
        p=self.selected_project()
        if not p or not self.guard_change():return
        ts=self.folder_threads(p.id)
        if not ts:return
        preview='\n'.join(t.title[:100] for t in ts[:8])+('\n외 '+str(len(ts)-8)+'개' if len(ts)>8 else '')
        if not confirm(self,'프로젝트 대화 연결 확정',p.name+f'\n\n폴더 기준 대화 {len(ts)}개의 소속을 이 프로젝트로 저장합니다.\n\n'+preview):return
        self.run_job('대화 소속 저장',lambda w:change_connections(self.adapter,p,{t.id:p.id for t in ts},self.journal),self.show_result)

    def require_linked(self,p):
        if not self.folder_threads(p.id):return True
        QMessageBox.information(self,'대화 연결 확인','폴더 기준으로 표시된 대화가 있습니다. 아래의 연결 확정 버튼으로 소속을 저장한 뒤 이동·정리하세요.');return False

    def show_transcript(self,item):
        tid=item.data(Qt.UserRole)
        title=item.text().split('\n')[0];bundle=self.active_bundle
        thread=next((t for t in self.snapshot.conversations if t.id==tid),None)
        def read(w):
            if bundle: return read_transcript(bundle,tid)
            if not thread or not thread.rollout: raise ValueError('대화 기록 경로가 없습니다.')
            return list(transcript(thread.rollout))
        def show(messages):
            d=QDialog(self);d.setWindowTitle(title);d.resize(790,650)
            layout=QVBoxLayout(d);text=QPlainTextEdit();text.setReadOnly(True)
            text.setPlainText('\n\n'.join(('나' if m['role']=='user' else 'Codex')+'\n'+m['text'] for m in messages));layout.addWidget(text);d.exec()
        self.run_job('대화 읽기',read,show)

    def backup(self,checked=False,projects=None,callback=None,retry_of=None):
        if self.shortcuts.busy():return
        projects=tuple(projects) if projects is not None else self.selected_projects()
        if not projects:return
        parent=QFileDialog.getExistingDirectory(self,'백업을 저장할 폴더')
        if not parent: return
        stamp=datetime.now(timezone(timedelta(hours=9))).strftime('%Y%m%d_%H%M%S')
        reserved=set();plans=[]
        for p in projects:
            safe=re.sub(r'[<>:"/\\|?*]','_',p.name).strip('. ') or 'project'
            destination=Path(parent)/(safe+'_'+stamp);n=2
            while destination.exists() or str(destination).casefold() in reserved:
                destination=Path(parent)/(safe+'_'+stamp+f' ({n})');n+=1
            reserved.add(str(destination).casefold());plans.append((p,destination))
        preview='\n\n'.join(f'{p.name} · 대화 {len(self.project_threads(p.id))}개 · 폴더 {len(p.roots)}개\n{destination}' for p,destination in plans)
        if not confirm(self,'프로젝트 백업',preview+'\n\n파일과 대화를 함께 복사하고 검증합니다. 원본은 유지합니다.'):return
        def done(result):
            report=self.journal.operation(result.report_id)
            verified=[Path(item['destinations'][0]) for item in report['payload']['items'] if item['status']=='completed']
            self.backups.extend(p for p in verified if p not in self.backups);self.save_settings()
            if verified:
                self.section_queries['backups']='';self.set_mode('backups')
                for n in range(self.project_list.count()):
                    if self.project_list.item(n).data(Qt.UserRole)==str(verified[-1]):self.project_list.setCurrentRow(n);break
            if callback:callback(result)
            else:
                self.open_activity().show_result(result);self.footer.setText(result.file_status)
        from .batch import run_backups
        adapter=self.adapter
        self.run_job('대화와 파일 백업',lambda worker:run_backups(adapter,plans,self.journal,export_project,worker.progress.emit,worker.cancelled.is_set,retry_of),done)

    def guard_change(self):
        try: self.adapter.ensure_write_allowed();return True
        except Exception as exc: QMessageBox.information(self,'연결 변경',str(exc));return False

    def move(self):
        sources=self.selected_projects()
        if not sources:return
        parent=QFileDialog.getExistingDirectory(self,'선택한 프로젝트를 옮길 폴더')
        if parent:self.transfer_projects(sources,parent=Path(parent))

    def merge(self):
        sources=self.selected_projects()
        if not sources:return
        others=[p for p in self.snapshot.projects if p.id not in {s.id for s in sources} and p.roots]
        if not others:QMessageBox.information(self,'합치기','선택하지 않은 대상 프로젝트가 필요합니다.');return
        from .naming import ProjectPicker
        dialog=ProjectPicker(self,others,'프로젝트 합치기',f'선택한 프로젝트 {len(sources)}개를 받을 프로젝트를 선택하세요.')
        if dialog.exec()==QDialog.Accepted:self.transfer_projects(sources,target=next(p for p in others if p.id==dialog.project_id()))

    def transfer_projects(self,sources,target=None,parent=None,done=None,retry_of=None):
        if self.shortcuts.busy() or not sources:return
        if target and (not target.roots or any(p.id==target.id for p in sources)):return
        from .batch import plan_projects,run_projects
        try:plans=plan_projects(sources,target.roots[0] if target else parent)
        except (ValueError,OSError) as exc:QMessageBox.warning(self,'이동할 수 없습니다',str(exc));return
        title='프로젝트 합치기' if target else '프로젝트 옮기기'
        inferred={p.id:tuple(t.id for t in self.folder_threads(p.id)) for p in sources}
        text=(f'대상 프로젝트: {target.name} (이 이름 유지)\n\n' if target else '')
        if target:text+='합친 뒤 원래 프로젝트 이름은 목록에서 사라지고, 대화는 대상 프로젝트에 모입니다.\n파일은 대상 폴더 아래 각 프로젝트 이름의 폴더로 모읍니다.\n\n'
        text+='\n\n'.join(p.name+'\n'+'\n'.join(str(a)+' → '+str(b) for a,b in mapping.items()) for p,mapping in plans)
        for p in sources:
            if inferred[p.id]:text+=f'\n\n{p.name}: 같은 폴더에서 작업한 대화 {len(inferred[p.id])}개를 먼저 연결합니다.'
        text+='\n\n대화와 작업 경로를 함께 변경합니다. 순서대로 처리하며 중단 시 완료한 작업은 유지됩니다.'
        clean=confirm_transfer(self,title,text)
        if clean is None or not self.guard_change():return
        adapter=self.adapter
        self.run_job(title,lambda worker:run_projects(adapter,plans,target,clean,self.journal,inferred,worker.progress.emit,worker.cancelled.is_set,retry_of),done or self.show_result)

    def start_transfer(self,p,destinations,target=None,done=None):
        text=p.name+(' → '+target.name if target else '')+'\n\n'
        text+='\n\n'.join(old+'\n→ '+str(new) for old,new in destinations.items())
        text+='\n\n대화의 소속과 현재 작업 경로를 함께 변경합니다.\n파일 복사와 연결을 다시 검증한 뒤 선택한 원본을 정리합니다.'
        inferred=tuple(t.id for t in self.folder_threads(p.id))
        if inferred:text+=f'\n\n같은 폴더에서 작업한 대화 {len(inferred)}개를 이 프로젝트에 연결한 다음 진행합니다.'
        clean=confirm_transfer(self,'합치기' if target else '옮기기',text)
        if clean is None: return
        recovery=(target.roots[0].parent if target else next(iter(destinations.values())).parent)/'.project-manager-recovery'
        from .flows import transfer_with_connections
        self.run_job('프로젝트를 옮기는 중' if not target else '프로젝트를 합치는 중',lambda w:transfer_with_connections(self.adapter,p,destinations,self.journal,recovery,target,clean,inferred,w.progress.emit,w.cancelled.is_set),done or self.show_result)

    def connections(self):
        p=self.selected_project()
        if not p or not self.guard_change(): return
        threads=self.project_threads(p.id)
        d=ConnectionsDialog(self,p,self.snapshot.projects,threads)
        if d.exec()!=QDialog.Accepted: return
        try: project,tids,target=d.values()
        except Exception as exc: QMessageBox.warning(self,'연결 관리',str(exc));return
        self.run_job('프로젝트 연결 갱신',lambda w:change_connections(self.adapter,project,{tid:target for tid in tids} if target!='unchanged' else {},self.journal),self.show_result)

    def export_cleanup(self):
        p=self.selected_project()
        if not p or not self.require_linked(p) or not self.guard_change(): return
        eligible=[]
        for bundle in self.backups:
            try:
                if load_manifest(bundle)['project']['id']==p.id and json.loads((bundle/'verification.json').read_text(encoding='utf-8')).get('restore_verified'): eligible.append(bundle)
            except (ValueError,OSError,TypeError,AttributeError): pass
        if not eligible: QMessageBox.information(self,'백업 후 정리','다른 저장소에서 복원 후 이어쓰기 확인을 마친 백업을 먼저 열어주세요.');return
        bundle=eligible[-1]
        if not confirm(self,'백업 후 원본 정리',p.name+'\n\n보존할 백업\n'+str(bundle)+'\n\n삭제할 원본 폴더\n'+'\n'.join(str(r) for r in p.roots)+'\n\n이 프로젝트의 대화와 원본 파일을 삭제합니다. 백업 이후 변경이 있으면 중단합니다.'): return
        from ..cleanup import cleanup_export
        self.run_job('백업을 확인하고 원본 정리',lambda w:cleanup_export(p,bundle,self.adapter,self.journal),self.show_result)

    def open_backup(self):
        folder=QFileDialog.getExistingDirectory(self,'Codex Manager로 만든 백업 폴더 선택')
        if not folder: return
        bundle=Path(folder)
        def done(check):
            if not check.ok: QMessageBox.warning(self,'백업 검증','\n'.join(check.errors));return
            if bundle not in self.backups: self.backups.append(bundle)
            self.save_settings()
            self.search.clear();self.set_mode('backups')
            for n in range(self.project_list.count()):
                if self.project_list.item(n).data(Qt.UserRole)==str(bundle):self.project_list.setCurrentRow(n);break
        self.run_job('백업 파일 검증',lambda w:verify_bundle(bundle),done)

    def restore(self):
        if not self.active_bundle or not self.guard_change(): return
        bundle=self.active_bundle
        try: m=load_manifest(bundle)
        except (ValueError,OSError) as exc: QMessageBox.warning(self,'백업 확인','백업을 다시 연결하거나 확인하세요.\n'+str(exc));return
        parent=QFileDialog.getExistingDirectory(self,'복원할 상위 폴더')
        if not parent: return
        destinations={r['id']:(Path(parent)/clean_path(r['original_path']).name if len(m['roots'])==1 else Path(parent)/r['id']/clean_path(r['original_path']).name) for r in m['roots']}
        if not confirm(self,'백업 가져오기',m['project']['name']+'\n\n'+'\n'.join(str(p) for p in destinations.values())+'\n\n파일을 복원하고 대화를 이 컴퓨터의 Codex에 등록합니다.'): return
        self.run_job('프로젝트 복원',lambda w:import_bundle(bundle,destinations,self.adapter,self.journal,w.progress.emit,w.cancelled.is_set),self.show_result)

    def show_recovery(self,operation_id=None):
        if self.shortcuts.busy():return
        if not isinstance(operation_id,str):
            item=self.project_list.currentItem()
            if not item:return
            operation_id=item.data(Qt.UserRole)
        data=next((p for p in self.recovery_items() if p['id']==operation_id),None)
        if not data:return
        recovery=data['payload'].get('recovery')
        events=self.journal.events(operation_id)
        error=next((e['payload']['error'] for e in reversed(events) if 'error' in e['payload']),'이전 상태로 되돌릴 수 있습니다.' if data['state']=='completed' else '작업이 중단됐습니다.')
        text='작업 상태: '+data['state']+'\n\n'+error+'\n\n'
        if data['payload']['kind']=='file-edit':text+=data['payload'].get('label','파일 작업')+'을 되돌립니다. 이후 변경된 파일은 덮어쓰지 않습니다.'
        elif data['payload']['kind'] in ('rename-thread','rename-threads'):text+='변경 전 대화 제목으로 되돌립니다.'
        elif recovery: text+='복구 사본\n'+recovery+'\n\n이전 위치와 연결을 복구합니다. 대상의 새 파일은 덮어쓰지 않습니다.'
        elif data['payload']['kind']=='connections': text+='이전 프로젝트 이름·폴더 연결·대화 소속을 복구합니다.'
        elif data['payload']['kind']=='export-cleanup': text+='백업 위치\n'+data['payload'].get('bundle','')+'\n\n정리 중 삭제된 대화와 파일을 백업에서 복구합니다. 남아 있는 대화와 변경된 파일은 보존합니다.'
        else: text+='백업 위치\n'+data['payload'].get('bundle','')+'\n\n중단된 가져오기를 이어서 검증합니다.'
        if not confirm(self,'작업 복구',text):return
        if data['payload']['kind']!='file-edit' and not self.guard_change(): return
        bundle_override=None
        if data['payload']['kind'] in ('import','export-cleanup'):
            selected=QFileDialog.getExistingDirectory(self,'중단된 작업의 백업 폴더 선택',data['payload'].get('bundle',''))
            if not selected: return
            bundle_override=Path(selected)
        self.run_job('프로젝트 복구',lambda w:recover_operation(operation_id,self.adapter,self.journal,w.progress.emit,bundle_override),self.show_result)

    def verify_restore(self):
        bundle=self.active_bundle
        if not bundle: return
        def done(check):
            if check.ok: QMessageBox.information(self,'복원 확인','복원 파일과 대화, 이어쓰기 완료를 확인했습니다.\n원래 컴퓨터에서 이 백업을 열면 원본 정리를 진행할 수 있습니다.')
            else: QMessageBox.information(self,'복원 확인','\n'.join(check.errors))
        self.run_job('복원 결과 확인',lambda w:verify_restored_bundle(bundle,self.adapter),done)

    def save_settings(self):
        path=self.state_dir/'settings.json'
        try:
            save_settings(path,{'home':str(self.adapter.home),'homes':[str(p) for p in self.stores],'backups':[str(p) for p in self.backups],'startup_update_check':self.startup_update_check})
            return True
        except OSError as exc:
            self.footer.setText('설정을 저장하지 못했습니다: '+str(exc));return False

    def assign_unassigned(self):
        item=self.project_list.currentItem()
        if not item or not self.guard_change(): return
        tid=item.data(Qt.UserRole)
        projects=list(self.snapshot.projects)
        if not projects: return
        labels=[p.name+' · '+p.id[:8] for p in projects]
        label,ok=QInputDialog.getItem(self,'대화 연결','대상 프로젝트',labels,0,False)
        if not ok: return
        p=projects[labels.index(label)]
        self.run_job('대화 연결',lambda w:change_connections(self.adapter,p,{tid:p.id},self.journal),self.show_result)

    def choose_home(self):
        if self.workers or self.updater.requested: self.footer.setText('진행 중인 작업을 마친 뒤 저장소를 바꿀 수 있습니다.');return
        path=QFileDialog.getExistingDirectory(self,'Codex 저장소 선택',str(self.adapter.home))
        if not path: return
        from ..codex.adapter import CodexAdapter
        self.connect_store(Path(path))

    def closeEvent(self,event):
        if self.workers:
            QMessageBox.information(self,'작업 중','진행 중인 작업을 취소하거나 완료한 뒤 닫으세요.');event.ignore()
        elif not self.stop_background():
            self._closing=True;event.ignore();QTimer.singleShot(100,self.close)
        elif not self.updater.close_when_idle(): event.ignore()
        else:
            self.save_settings()
            if hasattr(self,'taskbar_cleanup'):
                try:self.taskbar_cleanup()
                except OSError:pass
            event.accept()
