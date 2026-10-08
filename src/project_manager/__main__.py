import argparse
import os
import sys
from pathlib import Path
from PySide6.QtWidgets import QApplication,QMessageBox
from PySide6.QtCore import QLockFile
from PySide6.QtGui import QIcon
from .codex.adapter import CodexAdapter
from .ui.window import MainWindow
from .version import APP_NAME,VERSION
from .settings import read_settings


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--home',type=Path)
    parser.add_argument('--state-dir',type=Path,default=Path(os.environ.get('LOCALAPPDATA',str(Path.home())))/'ProjectConversationManager')
    parser.add_argument('--screenshot',type=Path)
    parser.add_argument('--no-update-check',action='store_true')
    parser.add_argument('--update-health',type=Path)
    parser.add_argument('--version',action='version',version=APP_NAME+' '+VERSION)
    args=parser.parse_args()
    if args.home is None:
        settings=args.state_dir/'settings.json'
        saved=read_settings(settings).get('home')
        if not isinstance(saved,str): saved=None
        args.home=Path(saved or os.environ.get('CODEX_HOME',str(Path.home()/'.codex')))
    if sys.platform=='win32':
        import ctypes
        set_id=ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID
        set_id.argtypes=[ctypes.c_wchar_p];set_id.restype=ctypes.c_long
        set_id('Contentrium.CodexManager')
    app=QApplication(sys.argv[:1]);app.setApplicationName(APP_NAME);app.setOrganizationName('Contentrium')
    app.setWindowIcon(QIcon(str(Path(__file__).parent/'ui/app-icon.ico')))
    try:
        args.state_dir.mkdir(parents=True,exist_ok=True)
        instance=QLockFile(str(args.state_dir/'application.lock'));instance.setStaleLockTime(0)
        if not instance.tryLock(0):
            QMessageBox.information(None,APP_NAME,'같은 설정을 사용하는 Codex Manager가 이미 실행 중입니다. 열린 프로그램을 사용하세요.');return 0
        window=MainWindow(CodexAdapter(args.home),args.state_dir,auto_refresh=not args.screenshot,start_update_check=not args.no_update_check)
    except Exception as exc:
        QMessageBox.critical(None,APP_NAME,'프로그램을 시작하지 못했습니다. 설정 폴더와 파일 상태를 확인하세요.\n'+str(exc));return 1
    window.show()
    if args.update_health:
        from .bundles import write_json
        from PySide6.QtCore import QTimer
        if not args.update_health.resolve().is_relative_to((args.state_dir/'updates').resolve()): raise ValueError('올바른 업데이트 확인 경로가 아닙니다.')
        QTimer.singleShot(500,lambda:write_json(args.update_health,{'version':VERSION,'install_dir':str(Path(sys.executable).parent.resolve()),'pid':os.getpid()}))
    if args.screenshot:
        window.refresh(window.adapter.snapshot(include_runtime=False));window.project_list.setCurrentRow(0)
        from PySide6.QtCore import QTimer
        def capture():
            args.screenshot.parent.mkdir(parents=True,exist_ok=True);window.grab().save(str(args.screenshot));window.close()
        QTimer.singleShot(300,capture)
    return app.exec()


if __name__=='__main__': raise SystemExit(main())
