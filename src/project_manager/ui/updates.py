import sys
import threading
import os
from pathlib import Path
from PySide6.QtCore import QObject,Signal,QTimer,QThread
from PySide6.QtGui import QDesktopServices
from PySide6.QtCore import QUrl
from PySide6.QtWidgets import QDialog,QVBoxLayout,QHBoxLayout,QLabel,QPushButton,QCheckBox,QPlainTextEdit,QProgressBar
from ..version import APP_NAME,VERSION,REPOSITORY_URL
from ..updates import check_release,download_and_stage,launch_installer,pending_updates,load_prepared,store_prepared


class UpdateChecker(QObject):
    checked=Signal(object);failed=Signal(str)
    def __init__(self,parent): super().__init__(parent);self.busy=False
    def check(self):
        if self.busy: return False
        self.busy=True
        def work():
            try:
                release=check_release();self.busy=False
                try: self.checked.emit(release)
                except RuntimeError: pass
            except Exception as exc:
                self.busy=False
                try: self.failed.emit(str(exc))
                except RuntimeError: pass
        threading.Thread(target=work,daemon=True).start()
        return True


class UpdateTask(QThread):
    progress=Signal(object,object);phase=Signal(str)
    def __init__(self,func):
        super().__init__();self.func=func;self.cancelled=threading.Event();self.value=None;self.error=None
    def run(self):
        try: self.value=self.func(self)
        except Exception as exc: self.error=str(exc)


class UpdateManager(QObject):
    changed=Signal();handoff=Signal()
    def __init__(self,window):
        super().__init__(window);self.window=window;self.release=None;self.plan=None
        self.state='idle';self.message='';self.current=0;self.total=0;self.requested=False;self._worker=None
        self._close_requested=False

    def prepare(self,release):
        if self.requested and self.state in ('launching','handoff'): return
        if not release or not release.available:
            self.release=None;self.plan=None;self.state='idle';self.requested=False
            if self._worker: self._worker.cancelled.set()
            self.changed.emit();return
        if not getattr(sys,'frozen',False): return
        if self.release and self.release.version==release.version and (self._worker or self.plan): return
        self.release=release
        if self._worker: return
        self.plan=None;self.state='downloading';self.message='업데이트를 미리 다운로드하고 검증합니다.';self.current=self.total=0
        def fetch(task):
            install=Path(sys.executable).parent
            plan=load_prepared(release,install,self.window.state_dir,task.cancelled.is_set)
            if plan: return plan
            plan=download_and_stage(release,install,self.window.state_dir,task.progress.emit,task.cancelled.is_set,task.phase.emit)
            if 'install_dir' in plan: store_prepared(plan)
            return plan
        self._start(fetch,'download',release.version)

    def _start(self,func,mode,version=None):
        task=UpdateTask(func);self._worker=task
        task.progress.connect(self._progress);task.phase.connect(self._phase)
        def finished():
            self._worker=None
            if task.error or task.cancelled.is_set():
                self.state='cancelled' if task.cancelled.is_set() else 'error'
                self.message='업데이트 준비를 취소했습니다.' if task.cancelled.is_set() else '업데이트 실패: '+task.error
                self.requested=False
            elif mode=='download':
                self.plan=task.value;self.state='ready';self.message='설치 준비 완료 · 버튼을 누르면 바로 설치·재시작합니다.'
            else:
                self.state='handoff';self.message='새 버전으로 다시 실행합니다.'
            task.deleteLater();self.changed.emit()
            if self._close_requested: self.window.close();return
            if self.state=='handoff': self.handoff.emit();return
            if mode=='download' and self.release and self.release.version!=version:
                newer=self.release;self.release=None;self.prepare(newer);return
            if self.requested and self.state=='ready': self._apply()
        task.finished.connect(finished);task.start();self.changed.emit()

    def _progress(self,current,total):
        self.current=current;self.total=total;self.changed.emit()

    def _phase(self,message):
        self.message=message;self.current=self.total=0;self.changed.emit()

    def request_install(self):
        if self.requested or self.state in ('handoff','cancelling') or self._close_requested: return
        if not self.release or not self.release.available: return
        self.requested=True
        if self.state in ('idle','error','cancelled'):
            release=self.release;self.release=None;self.prepare(release)
        elif self.plan and not self._worker: self._apply()
        self.changed.emit()

    def _apply(self):
        if self.window.workers or not self.window.stop_background():
            self.state='waiting';self.message='진행 중인 작업을 마무리하면 자동으로 설치합니다.';self.changed.emit()
            QTimer.singleShot(100,self._resume);return
        if not self.window.save_settings():
            self.state='error';self.message=self.window.footer.text();self.requested=False;self.changed.emit();return
        self.state='launching';self.message='설치 준비를 확인합니다. 잠시 후 자동으로 다시 실행합니다.'
        self.plan['parent_pid']=os.getpid()
        self._start(lambda task:launch_installer(self.plan),'install')

    def _resume(self):
        if self.requested and self.state=='waiting': self._apply()

    def cancel(self):
        if self.state in ('launching','handoff'): return
        self.requested=False
        if self._worker:
            self._worker.cancelled.set();self.state='cancelling';self.message='업데이트 준비를 중단합니다…';self.changed.emit()
        else: self.state='ready' if self.plan else 'cancelled';self.changed.emit()

    def close_when_idle(self):
        if self._worker:
            if self.state=='launching': return False
            self._close_requested=True;self.cancel();return False
        return True


class UpdateDialog(QDialog):
    def __init__(self,window):
        super().__init__(window);self.window=window;self.setWindowTitle(APP_NAME+' 업데이트');self.resize(650,460)
        layout=QVBoxLayout(self)
        self.current=QLabel('현재 버전  '+VERSION);self.current.setStyleSheet('font-size: 18px; font-weight: 600;');layout.addWidget(self.current)
        self.status=QLabel('최신 버전을 확인할 수 있습니다.');self.status.setWordWrap(True);layout.addWidget(self.status)
        self.notes=QPlainTextEdit();self.notes.setReadOnly(True);self.notes.setPlaceholderText('새 버전의 변경 내용');layout.addWidget(self.notes,1)
        self.progress=QProgressBar();self.progress.hide();layout.addWidget(self.progress)
        self.startup=QCheckBox('프로그램을 시작할 때 업데이트 확인');self.startup.setChecked(window.startup_update_check)
        self.startup.toggled.connect(self.change_startup);layout.addWidget(self.startup)
        if pending_updates(window.state_dir):
            recovery=QPushButton('중단된 업데이트 복구 위치 열기');layout.addWidget(recovery)
            recovery.clicked.connect(lambda:QDesktopServices.openUrl(QUrl.fromLocalFile(str(window.state_dir/'updates'))))
            layout.addWidget(QLabel('프로그램을 닫고 해당 폴더의 recover-*.cmd를 실행하면 이전 버전으로 복구합니다.'))
        info=QLabel('설치하면 프로그램을 종료하고 새 버전으로 다시 실행합니다.\n대화·작업 폴더·백업·설정은 유지합니다.');info.setObjectName('muted');layout.addWidget(info)
        buttons=QHBoxLayout();self.check=QPushButton('업데이트 확인');self.check.clicked.connect(lambda:window.check_updates(False));buttons.addWidget(self.check)
        release=QPushButton('GitHub 배포 페이지');release.clicked.connect(lambda:QDesktopServices.openUrl(QUrl(REPOSITORY_URL+'/releases')));buttons.addWidget(release);buttons.addStretch()
        self.install=QPushButton('업데이트 설치·재시작');self.install.setObjectName('primary');self.install.setEnabled(False);self.install.clicked.connect(self.install_update);buttons.addWidget(self.install);layout.addLayout(buttons)
        self.cancel_button=QPushButton('준비 취소');self.cancel_button.hide();self.cancel_button.clicked.connect(window.updater.cancel);buttons.insertWidget(2,self.cancel_button)
        window.updater.changed.connect(self.show_preparation)

    def change_startup(self,value):
        self.window.startup_update_check=value;self.window.save_settings()

    def show_release(self,release):
        self.install.setEnabled(bool(release and release.available and getattr(sys,'frozen',False)))
        if release is None: self.status.setText('아직 공개된 업데이트가 없습니다.');return
        self.status.setText('새 버전 '+release.version+'을 설치할 수 있습니다.' if release.available else '최신 버전을 사용하고 있습니다. ('+release.version+')')
        self.notes.setPlainText(release.notes)
        if release.available and not getattr(sys,'frozen',False): self.status.setText('새 버전 '+release.version+' · 실행 파일 배포에서 설치할 수 있습니다.')
        self.show_preparation()

    def show_preparation(self):
        updater=self.window.updater
        if updater.state=='idle' or not updater.release:
            self.progress.hide();self.cancel_button.hide();self.check.setEnabled(True);self.install.setEnabled(False);return
        self.status.setText(updater.message)
        busy=updater._worker is not None or updater.state=='waiting'
        self.progress.setVisible(busy)
        if updater.total:
            self.progress.setRange(0,1000);self.progress.setValue(int(updater.current/updater.total*1000))
        else: self.progress.setRange(0,0)
        self.cancel_button.setVisible(updater.state in ('downloading','waiting'))
        self.install.setEnabled(bool(updater.release and updater.release.available and not updater.requested and updater.state not in ('handoff','cancelling') and not updater._close_requested))
        self.install.setText('지금 업데이트' if updater.state=='ready' else '다시 시도' if updater.state in ('error','cancelled') else '업데이트 설치·재시작')
        self.check.setEnabled(not busy and not updater.requested)

    def install_update(self):
        if getattr(sys,'frozen',False): self.window.updater.request_install()
