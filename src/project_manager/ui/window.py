import json
import re
import threading
from datetime import datetime,timezone,timedelta
from pathlib import Path
from PySide6.QtCore import Qt,QThread,Signal,QUrl,QTimer
from PySide6.QtGui import QDesktopServices
from PySide6.QtWidgets import (QMainWindow,QWidget,QHBoxLayout,QVBoxLayout,QLabel,QPushButton,
    QLineEdit,QListWidget,QListWidgetItem,QSplitter,QFileDialog,QMessageBox,QProgressDialog,
    QDialog,QPlainTextEdit,QInputDialog,QAbstractItemView)
from ..models import Snapshot
from ..catalog import clean_path
from ..codex.portability import transcript
from ..bundles import export_project,verify_bundle,load_manifest,read_transcript
from ..files import scan_roots
from ..journal import Journal
from ..operations import transfer_project,import_bundle,recover_operation,change_connections
from ..restore_proof import verify_restored_bundle
from .dialogs import confirm,confirm_transfer,ConnectionsDialog
from .updates import UpdateChecker,UpdateDialog,UpdateManager
from ..version import APP_NAME,VERSION
from ..settings import read_settings,save_settings


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


class MainWindow(QMainWindow):
    def __init__(self,adapter,state_dir,auto_refresh=True,start_update_check=True):
        super().__init__();self.adapter=adapter;self.state_dir=Path(state_dir)
        self.journal=Journal(self.state_dir/'journal.sqlite');self.snapshot=Snapshot((),(),'')
        self.mode='projects';self.workers=[];self.sizes={};self.backups=[];self.active_bundle=None
        self.startup_update_check=True;self.latest_release=None;self.update_dialog=None;self.update_popup_pending=False
        settings_path=self.state_dir/'settings.json'
        if settings_path.exists():
            try:
                saved=read_settings(settings_path)
                backups=saved.get('backups',[])
                self.backups=[Path(p) for p in backups if isinstance(p,str) and p] if isinstance(backups,list) else []
                self.startup_update_check=saved.get('startup_update_check',True) is not False
            except (ValueError,OSError): pass
        self.setWindowTitle(APP_NAME);self.resize(1260,810);self.setMinimumSize(950,630)
        self.setStyleSheet((Path(__file__).parent/'theme.qss').read_text(encoding='utf-8'))
        base=QWidget();self.setCentralWidget(base);outer=QHBoxLayout(base);outer.setContentsMargins(0,0,0,0);outer.setSpacing(0)
        sidebar=QWidget();sidebar.setObjectName('sidebar');sidebar.setFixedWidth(205)
        nav=QVBoxLayout(sidebar);nav.setContentsMargins(16,20,16,20)
        brand=QLabel(APP_NAME);brand.setObjectName('brand');nav.addWidget(brand)
        sub=QLabel('대화와 폴더를 함께');sub.setObjectName('muted');nav.addWidget(sub);nav.addSpacing(24)
        self.nav_buttons=[]
        for label,mode in [('프로젝트','projects'),('연결 안 된 대화','unassigned'),('백업 보관함','backups'),('진행·복구','recovery')]:
            b=QPushButton(label);b.setObjectName('nav');b.setCheckable(True);b.clicked.connect(lambda _,m=mode:self.set_mode(m));nav.addWidget(b);self.nav_buttons.append((b,mode))
        nav.addStretch();self.connection=QLabel('로컬 Codex');self.connection.setObjectName('muted');self.connection.setWordWrap(True);nav.addWidget(self.connection)
        settings=QPushButton('저장소 위치');settings.clicked.connect(self.choose_home);nav.addWidget(settings)
        self.update_button=QPushButton('업데이트');self.update_button.clicked.connect(lambda:self.open_updates());nav.addWidget(self.update_button)
        version=QLabel('v'+VERSION+' · CONTENTRIUM');version.setObjectName('muted');nav.addWidget(version);outer.addWidget(sidebar)
        content=QVBoxLayout();content.setContentsMargins(28,25,25,16);content.setSpacing(18);outer.addLayout(content,1)
        top=QHBoxLayout();self.heading=QLabel('프로젝트');self.heading.setObjectName('heading');top.addWidget(self.heading);top.addStretch()
        self.search=QLineEdit();self.search.setPlaceholderText('프로젝트·대화 검색');self.search.setMaximumWidth(270);self.search.textChanged.connect(self.filter_list);top.addWidget(self.search)
        refresh=QPushButton('새로고침');refresh.clicked.connect(self.reload);top.addWidget(refresh)
        add=QPushButton('백업 가져오기');add.setObjectName('primary');add.clicked.connect(self.open_backup);top.addWidget(add);content.addLayout(top)
        self.subtitle=QLabel('프로젝트를 선택하면 연결된 대화와 폴더를 확인할 수 있습니다.');self.subtitle.setObjectName('muted');self.subtitle.setWordWrap(True);content.addWidget(self.subtitle)
        actions=QHBoxLayout();self.buttons={}
        for label,key,callback in [('옮기기','move',self.move),('합치기','merge',self.merge),('연결 관리','links',self.connections),('백업','backup',self.backup),('백업 후 정리','cleanup',self.export_cleanup)]:
            b=QPushButton(label);b.clicked.connect(callback);actions.addWidget(b);self.buttons[key]=b
        actions.addStretch();content.addLayout(actions)
        self.move_button=self.buttons['move'];self.cleanup_button=self.buttons['cleanup']
        split=QSplitter(Qt.Horizontal);content.addWidget(split,1)
        self.project_list=QListWidget();self.project_list.currentItemChanged.connect(self.show_detail);split.addWidget(self.project_list)
        detail=QWidget();detail_layout=QVBoxLayout(detail);detail_layout.setContentsMargins(23,0,0,0)
        self.detail_title=QLabel('프로젝트 선택');self.detail_title.setStyleSheet('font-size: 18px; font-weight: 600;');detail_layout.addWidget(self.detail_title)
        folder_title=QHBoxLayout();folder_title.addWidget(QLabel('연결 폴더'));folder_title.addStretch()
        size=QPushButton('용량 계산');size.clicked.connect(self.calculate_size);folder_title.addWidget(size);detail_layout.addLayout(folder_title)
        self.folder_list=QListWidget();self.folder_list.setMaximumHeight(135);self.folder_list.itemDoubleClicked.connect(lambda item:QDesktopServices.openUrl(QUrl.fromLocalFile(item.data(Qt.UserRole))));detail_layout.addWidget(self.folder_list)
        detail_layout.addWidget(QLabel('대화'));self.thread_list=QListWidget();self.thread_list.setSelectionMode(QAbstractItemView.ExtendedSelection);self.thread_list.itemDoubleClicked.connect(self.show_transcript);detail_layout.addWidget(self.thread_list,1)
        self.detail_status=QLabel('');self.detail_status.setObjectName('muted');self.detail_status.setWordWrap(True);detail_layout.addWidget(self.detail_status)
        self.bundle_action=QPushButton('선택한 백업 복원');self.bundle_action.clicked.connect(self.restore);self.bundle_action.hide();detail_layout.addWidget(self.bundle_action)
        self.proof_action=QPushButton('복원 후 이어쓰기 확인');self.proof_action.clicked.connect(self.verify_restore);self.proof_action.hide();detail_layout.addWidget(self.proof_action)
        self.recovery_action=QPushButton('복구 정보 보기');self.recovery_action.clicked.connect(self.show_recovery);self.recovery_action.hide();detail_layout.addWidget(self.recovery_action)
        self.assign_action=QPushButton('프로젝트에 연결');self.assign_action.clicked.connect(self.assign_unassigned);self.assign_action.hide();detail_layout.addWidget(self.assign_action)
        split.addWidget(detail);split.setSizes([470,510])
        self.footer=QLabel('원본 파일과 대화를 함께 관리합니다.');self.footer.setObjectName('muted');content.addWidget(self.footer)
        self.set_mode('projects')
        self.updater=UpdateManager(self);self.updater.handoff.connect(self.close)
        self.updater.changed.connect(lambda:self.centralWidget().setEnabled(not (self.updater.requested or self.updater._close_requested)))
        self.update_checker=UpdateChecker(self);self.update_checker.checked.connect(self.update_checked);self.update_checker.failed.connect(self.update_failed)
        if auto_refresh: QTimer.singleShot(0,self.reload)
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
        p=self.selected_project();return (p.id,) if p else ()

    def set_mode(self,mode):
        self.mode=mode
        for b,m in self.nav_buttons: b.setChecked(m==mode)
        self.heading.setText({'projects':'프로젝트','unassigned':'연결 안 된 대화','backups':'백업 보관함','recovery':'진행·복구'}[mode])
        self.filter_list()

    def refresh(self,snapshot):
        self.snapshot=snapshot;self.filter_list()

    def reload(self):
        self.run_job('프로젝트 확인',lambda w:self.adapter.snapshot(include_runtime=False),self.refresh)

    def filter_list(self):
        query=self.search.text().lower();selected=self.project_list.currentItem()
        old=selected.data(Qt.UserRole) if selected else None
        self.project_list.clear()
        if self.mode=='projects':
            projects=sorted(self.snapshot.projects,key=lambda p:(not p.name.lower().startswith('test '),p.name.casefold()))
            for p in projects:
                ts=[t for t in self.snapshot.conversations if t.project_id==p.id]
                if query and query not in p.name.lower() and not any(query in t.title.lower() for t in ts): continue
                item=QListWidgetItem(f'{p.name}\n{len(ts)}개 대화  ·  {len(p.roots)}개 폴더  ·  {bytes_text(self.sizes.get(p.id))}')
                item.setData(Qt.UserRole,p.id);self.project_list.addItem(item)
                if p.id==old: self.project_list.setCurrentItem(item)
        elif self.mode=='unassigned':
            for t in self.snapshot.conversations:
                if t.project_id is None and (not query or query in t.title.lower()):
                    item=QListWidgetItem(t.title+'\n'+str(t.cwd));item.setData(Qt.UserRole,t.id);self.project_list.addItem(item)
        elif self.mode=='backups':
            for bundle in self.backups:
                try: m=load_manifest(bundle);name=m['project']['name']
                except Exception: name='연결되지 않은 백업' if not bundle.exists() else '확인 필요한 백업'
                item=QListWidgetItem(name+'\n'+str(bundle));item.setData(Qt.UserRole,str(bundle));self.project_list.addItem(item)
        else:
            for operation in self.journal.pending():
                item=QListWidgetItem(operation['payload'].get('kind','작업')+' · '+operation['state']+'\n'+operation['id']);item.setData(Qt.UserRole,operation['id']);self.project_list.addItem(item)
        self.show_detail()

    def show_detail(self,*_):
        self.folder_list.clear();self.thread_list.clear();self.detail_status.setText('');self.active_bundle=None
        self.bundle_action.setVisible(self.mode=='backups');self.proof_action.setVisible(self.mode=='backups');self.recovery_action.setVisible(self.mode=='recovery')
        self.assign_action.setVisible(self.mode=='unassigned')
        p=self.selected_project()
        for b in self.buttons.values(): b.setEnabled(p is not None)
        self.cleanup_button.setEnabled(False);self.cleanup_button.setToolTip('실제 계정의 복원 후 이어쓰기 검증 전에는 원본 정리를 사용하지 않습니다.')
        if p:
            for bundle in self.backups:
                try:
                    m=load_manifest(bundle);proof=json.loads((bundle/'verification.json').read_text(encoding='utf-8'))
                    if m['project']['id']==p.id and proof.get('restore_verified'): self.cleanup_button.setEnabled(True)
                except (ValueError,OSError,TypeError,AttributeError): pass
            self.detail_title.setText(p.name)
            for path in p.roots:
                item=QListWidgetItem(str(path)+('' if path.exists() else '\n폴더 없음'));item.setToolTip(str(path));item.setData(Qt.UserRole,str(path));self.folder_list.addItem(item)
            for t in self.snapshot.conversations:
                if t.project_id==p.id:
                    label=t.title+('  ·  보관됨' if t.archived else '')+'\n'+str(t.cwd)
                    item=QListWidgetItem(label);item.setToolTip(str(t.cwd));item.setData(Qt.UserRole,t.id);self.thread_list.addItem(item)
            self.detail_status.setText('대화를 두 번 클릭하면 내용을 볼 수 있습니다.\n폴더를 두 번 클릭하면 탐색기로 엽니다.')
        else:
            self.detail_title.setText('항목 선택')
            item=self.project_list.currentItem()
            if item and self.mode=='unassigned':
                tid=item.data(Qt.UserRole);t=next(t for t in self.snapshot.conversations if t.id==tid)
                self.detail_title.setText(t.title);i=QListWidgetItem(t.title);i.setData(Qt.UserRole,tid);self.thread_list.addItem(i)
            if item and self.mode=='backups':
                self.active_bundle=Path(item.data(Qt.UserRole))
                try:
                    m=load_manifest(self.active_bundle);self.detail_title.setText(m['project']['name'])
                    self.bundle_action.setEnabled(True);self.proof_action.setEnabled(True)
                    for r in m['roots']: self.folder_list.addItem(r['original_path'])
                    for t in m['conversations']:
                        i=QListWidgetItem(t['title']);i.setData(Qt.UserRole,t['id']);self.thread_list.addItem(i)
                    self.detail_status.setText('대화가 포함된 백업\n복원 전에 파일 무결성과 경로를 검사합니다.')
                except Exception as exc:
                    self.detail_status.setText('백업을 연결하거나 파일 상태를 확인하세요.\n'+str(exc))
                    self.bundle_action.setEnabled(False);self.proof_action.setEnabled(False)

    def run_job(self,title,func,done=None):
        if self.updater.requested:
            self.footer.setText('업데이트 설치가 진행 중입니다. 완료 후 작업할 수 있습니다.');return
        if self.workers: return
        dialog=QProgressDialog(title,'취소',0,0,self);dialog.setWindowTitle('작업 중');dialog.setWindowModality(Qt.WindowModal);dialog.setMinimumDuration(0);dialog.setAutoClose(False);dialog.setAutoReset(False)
        worker=Worker(func);self.workers.append(worker);dialog.canceled.connect(worker.cancelled.set)
        def progress(current,total):
            if total: dialog.setRange(0,1000);dialog.setValue(int(current/total*1000));dialog.setLabelText(f'{title}\n{bytes_text(current)} / {bytes_text(total)}')
        worker.progress.connect(progress)
        def success(result):
            dialog.close()
            if done: done(result)
        worker.result.connect(success);worker.error.connect(lambda e:(dialog.close(),QMessageBox.warning(self,'작업 확인',e)))
        def finish():
            self.workers.remove(worker);worker.deleteLater()
        worker.finished.connect(finish);worker.start()

    def show_result(self,result):
        if self.updater.requested:
            self.footer.setText('프로젝트 작업 완료 · 업데이트를 계속합니다.');return
        if hasattr(result,'state'):
            message='\n'.join([result.file_status,result.codex_status,result.cleanup_status,result.mobile_status,*result.errors])
            QMessageBox.information(self,'작업 결과',message)
        else: QMessageBox.information(self,'작업 결과','검증을 마쳤습니다.')
        QTimer.singleShot(50,self.reload)

    def calculate_size(self):
        p=self.selected_project()
        if not p: return
        def done(inv):
            self.sizes[p.id]=inv.logical_bytes;self.filter_list()
            self.detail_status.setText(bytes_text(inv.logical_bytes)+(('\n확인 필요: '+ '\n'.join(inv.blockers[:4])) if inv.blockers else ''))
        self.run_job('폴더 용량 계산',lambda w:scan_roots({str(i):r for i,r in enumerate(p.roots)},False),done)

    def show_transcript(self,item):
        tid=item.data(Qt.UserRole)
        def read(w):
            if self.active_bundle: return read_transcript(self.active_bundle,tid)
            t=next(t for t in self.snapshot.conversations if t.id==tid)
            if not t.rollout: raise ValueError('대화 기록 경로가 없습니다.')
            return list(transcript(t.rollout))
        def show(messages):
            d=QDialog(self);d.setWindowTitle(item.text().split('\n')[0]);d.resize(790,650)
            layout=QVBoxLayout(d);text=QPlainTextEdit();text.setReadOnly(True)
            text.setPlainText('\n\n'.join(('나' if m['role']=='user' else 'Codex')+'\n'+m['text'] for m in messages));layout.addWidget(text);d.exec()
        self.run_job('대화 읽기',read,show)

    def backup(self):
        p=self.selected_project()
        if not p: return
        parent=QFileDialog.getExistingDirectory(self,'백업을 저장할 폴더')
        if not parent: return
        stamp=datetime.now(timezone(timedelta(hours=9))).strftime('%Y%m%d_%H%M%S')
        safe=re.sub(r'[<>:"/\\|?*]','_',p.name).strip('. ') or 'project'
        destination=Path(parent)/(safe+'_'+stamp)
        ts=[t for t in self.snapshot.conversations if t.project_id==p.id]
        if not confirm(self,'프로젝트 백업',f'{p.name}\n\n대화 {len(ts)}개와 폴더 {len(p.roots)}개를 함께 복사합니다.\n\n백업 위치\n{destination}\n\n원본은 유지합니다.'): return
        def done(result):
            self.backups.append(destination);self.save_settings();self.footer.setText('백업 저장: '+str(destination));self.show_result(result)
        self.run_job('대화와 파일 백업',lambda w:export_project(p,self.adapter.snapshot(),destination,w.progress.emit,w.cancelled.is_set),done)

    def guard_change(self):
        try: self.adapter.ensure_write_allowed();return True
        except Exception as exc: QMessageBox.information(self,'연결 변경',str(exc));return False

    def move(self):
        p=self.selected_project()
        if not p or not self.guard_change(): return
        parent=QFileDialog.getExistingDirectory(self,'프로젝트를 옮길 상위 폴더')
        if not parent: return
        safe=re.sub(r'[<>:"/\\|?*]','_',p.name).strip('. ') or 'project'
        destinations={str(r):(Path(parent)/r.name if len(p.roots)==1 else Path(parent)/safe/f'root-{i+1:02d}'/r.name) for i,r in enumerate(p.roots)}
        self.start_transfer(p,destinations)

    def merge(self):
        p=self.selected_project()
        if not p or not self.guard_change(): return
        others=[q for q in self.snapshot.projects if q.id!=p.id and q.roots]
        if not others: QMessageBox.information(self,'합치기','합칠 대상 프로젝트가 없습니다.');return
        labels=[q.name+' · '+str(q.roots[0]) for q in others]
        label,ok=QInputDialog.getItem(self,'프로젝트 합치기','대상 프로젝트',labels,0,False)
        if not ok: return
        target=others[labels.index(label)]
        safe=re.sub(r'[<>:"/\\|?*]','_',p.name).strip('. ') or 'project'
        destinations={str(r):target.roots[0]/safe/(r.name if len(p.roots)>1 else '') for r in p.roots}
        self.start_transfer(p,destinations,target)

    def start_transfer(self,p,destinations,target=None):
        text=p.name+(' → '+target.name if target else '')+'\n\n'
        text+='\n\n'.join(old+'\n→ '+str(new) for old,new in destinations.items())
        text+='\n\n대화의 소속과 현재 작업 경로를 함께 변경합니다.\n파일 복사와 연결을 다시 검증한 뒤 선택한 원본을 정리합니다.'
        clean=confirm_transfer(self,'합치기' if target else '옮기기',text)
        if clean is None: return
        recovery=(target.roots[0].parent if target else next(iter(destinations.values())).parent)/'.project-manager-recovery'
        self.run_job('파일과 대화 연결 변경',lambda w:transfer_project(self.adapter,p,destinations,self.journal,recovery,target,clean,w.progress.emit,w.cancelled.is_set),self.show_result)

    def connections(self):
        p=self.selected_project()
        if not p or not self.guard_change(): return
        threads=[t for t in self.snapshot.conversations if t.project_id==p.id]
        d=ConnectionsDialog(self,p,self.snapshot.projects,threads)
        if d.exec()!=QDialog.Accepted: return
        try: project,tids,target=d.values()
        except Exception as exc: QMessageBox.warning(self,'연결 관리',str(exc));return
        self.run_job('프로젝트 연결 갱신',lambda w:change_connections(self.adapter,project,{tid:target for tid in tids} if target!='unchanged' else {},self.journal),self.show_result)

    def export_cleanup(self):
        p=self.selected_project()
        if not p or not self.guard_change(): return
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
        folder=QFileDialog.getExistingDirectory(self,'manifest.json이 있는 백업 폴더')
        if not folder: return
        bundle=Path(folder)
        def done(check):
            if not check.ok: QMessageBox.warning(self,'백업 검증','\n'.join(check.errors));return
            if bundle not in self.backups: self.backups.append(bundle)
            self.save_settings()
            self.set_mode('backups');self.project_list.setCurrentRow(self.backups.index(bundle))
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

    def show_recovery(self):
        item=self.project_list.currentItem()
        if not item: return
        operation_id=item.data(Qt.UserRole)
        data=next(p for p in self.journal.pending() if p['id']==operation_id)
        recovery=data['payload'].get('recovery')
        events=self.journal.events(operation_id)
        error=next((e['payload']['error'] for e in reversed(events) if 'error' in e['payload']),'작업이 중단됐습니다.')
        text='작업 상태: '+data['state']+'\n\n'+error+'\n\n'
        if recovery: text+='복구 사본\n'+recovery+'\n\n이전 위치와 연결을 복구합니다. 대상의 새 파일은 덮어쓰지 않습니다.'
        elif data['payload']['kind']=='connections': text+='이전 프로젝트 이름·폴더 연결·대화 소속을 복구합니다.'
        elif data['payload']['kind']=='export-cleanup': text+='백업 위치\n'+data['payload'].get('bundle','')+'\n\n정리 중 삭제된 대화와 파일을 백업에서 복구합니다. 남아 있는 대화와 변경된 파일은 보존합니다.'
        else: text+='백업 위치\n'+data['payload'].get('bundle','')+'\n\n중단된 가져오기를 이어서 검증합니다.'
        if not confirm(self,'작업 복구',text) or not self.guard_change(): return
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
            save_settings(path,{'home':str(self.adapter.home),'backups':[str(p) for p in self.backups],'startup_update_check':self.startup_update_check})
            return True
        except OSError as exc:
            self.footer.setText('설정을 저장하지 못했습니다: '+str(exc));return False

    def assign_unassigned(self):
        item=self.project_list.currentItem()
        if not item or not self.guard_change(): return
        projects=list(self.snapshot.projects)
        if not projects: return
        labels=[p.name+' · '+p.id[:8] for p in projects]
        label,ok=QInputDialog.getItem(self,'대화 연결','대상 프로젝트',labels,0,False)
        if not ok: return
        p=projects[labels.index(label)];tid=item.data(Qt.UserRole)
        self.run_job('대화 연결',lambda w:change_connections(self.adapter,p,{tid:p.id},self.journal),self.show_result)

    def choose_home(self):
        if self.workers or self.updater.requested: self.footer.setText('진행 중인 작업을 마친 뒤 저장소를 바꿀 수 있습니다.');return
        path=QFileDialog.getExistingDirectory(self,'Codex 저장소 선택',str(self.adapter.home))
        if not path: return
        from ..codex.adapter import CodexAdapter
        self.adapter=CodexAdapter(Path(path));self.connection.setText(str(path));self.reload()

    def closeEvent(self,event):
        if self.workers:
            QMessageBox.information(self,'작업 중','진행 중인 작업을 취소하거나 완료한 뒤 닫으세요.');event.ignore()
        elif not self.updater.close_when_idle(): event.ignore()
        else: self.save_settings();event.accept()
