import sys
import threading
from pathlib import Path
from PySide6.QtCore import QObject,Signal,QTimer
from PySide6.QtGui import QDesktopServices
from PySide6.QtCore import QUrl
from PySide6.QtWidgets import QDialog,QVBoxLayout,QHBoxLayout,QLabel,QPushButton,QCheckBox,QPlainTextEdit
from ..version import APP_NAME,VERSION,REPOSITORY_URL
from ..updates import check_release,download_and_stage,launch_installer,pending_updates


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


class UpdateDialog(QDialog):
    def __init__(self,window):
        super().__init__(window);self.window=window;self.setWindowTitle(APP_NAME+' 업데이트');self.resize(650,460)
        layout=QVBoxLayout(self)
        self.current=QLabel('현재 버전  '+VERSION);self.current.setStyleSheet('font-size: 18px; font-weight: 600;');layout.addWidget(self.current)
        self.status=QLabel('최신 버전을 확인할 수 있습니다.');self.status.setWordWrap(True);layout.addWidget(self.status)
        self.notes=QPlainTextEdit();self.notes.setReadOnly(True);self.notes.setPlaceholderText('새 버전의 변경 내용');layout.addWidget(self.notes,1)
        self.startup=QCheckBox('프로그램을 시작할 때 업데이트 확인');self.startup.setChecked(window.startup_update_check)
        self.startup.toggled.connect(self.change_startup);layout.addWidget(self.startup)
        if pending_updates(window.state_dir):
            recovery=QPushButton('중단된 업데이트 복구 위치 열기');layout.addWidget(recovery)
            recovery.clicked.connect(lambda:QDesktopServices.openUrl(QUrl.fromLocalFile(str(window.state_dir/'updates'))))
            layout.addWidget(QLabel('프로그램을 닫고 해당 폴더의 recover-*.cmd를 실행하면 이전 버전으로 복구합니다.'))
        info=QLabel('설치하면 프로그램을 종료하고 새 버전으로 다시 실행합니다.\n대화·작업 폴더·백업·설정은 유지합니다.');info.setObjectName('muted');layout.addWidget(info)
        buttons=QHBoxLayout();check=QPushButton('업데이트 확인');check.clicked.connect(lambda:window.check_updates(False));buttons.addWidget(check)
        release=QPushButton('GitHub 배포 페이지');release.clicked.connect(lambda:QDesktopServices.openUrl(QUrl(REPOSITORY_URL+'/releases')));buttons.addWidget(release);buttons.addStretch()
        self.install=QPushButton('업데이트 설치·재시작');self.install.setObjectName('primary');self.install.setEnabled(False);self.install.clicked.connect(self.install_update);buttons.addWidget(self.install);layout.addLayout(buttons)

    def change_startup(self,value):
        self.window.startup_update_check=value;self.window.save_settings()

    def show_release(self,release):
        self.install.setEnabled(bool(release and release.available and getattr(sys,'frozen',False)))
        if release is None: self.status.setText('아직 공개된 업데이트가 없습니다.');return
        self.status.setText('새 버전 '+release.version+'을 설치할 수 있습니다.' if release.available else '최신 버전을 사용하고 있습니다. ('+release.version+')')
        self.notes.setPlainText(release.notes)
        if release.available and not getattr(sys,'frozen',False): self.status.setText('새 버전 '+release.version+' · 실행 파일 배포에서 설치할 수 있습니다.')

    def install_update(self):
        window=self.window;release=window.latest_release
        if not release or not release.available or not getattr(sys,'frozen',False): return
        if window.workers: self.status.setText('진행 중인 작업을 마친 뒤 업데이트하세요.');return
        self.install.setEnabled(False);self.hide()
        def prepared(plan):
            window.save_settings()
            def apply_when_idle():
                if window.workers: QTimer.singleShot(100,apply_when_idle);return
                try: launch_installer(plan)
                except Exception as exc:
                    self.show();self.show_release(release);self.status.setText('설치 준비 실패: '+str(exc));return
                window.close()
            QTimer.singleShot(100,apply_when_idle)
        window.run_job('업데이트 다운로드·검증',lambda worker:download_and_stage(release,Path(sys.executable).parent,window.state_dir,worker.progress.emit,worker.cancelled.is_set),prepared)
