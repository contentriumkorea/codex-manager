import argparse
import os
import sys
from pathlib import Path
from PySide6.QtWidgets import QApplication
from .codex.adapter import CodexAdapter
from .ui.window import MainWindow
from .version import APP_NAME,VERSION


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
        import json
        settings=args.state_dir/'settings.json'
        try: saved=json.loads(settings.read_text(encoding='utf-8')).get('home') if settings.exists() else None
        except (ValueError,OSError): saved=None
        args.home=Path(saved or os.environ.get('CODEX_HOME',str(Path.home()/'.codex')))
    app=QApplication(sys.argv[:1]);app.setApplicationName(APP_NAME);app.setOrganizationName('Contentrium')
    window=MainWindow(CodexAdapter(args.home),args.state_dir,auto_refresh=not args.screenshot,start_update_check=not args.no_update_check)
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
            args.screenshot.parent.mkdir(parents=True,exist_ok=True);window.grab().save(str(args.screenshot));app.quit()
        QTimer.singleShot(300,capture)
    return app.exec()


if __name__=='__main__': raise SystemExit(main())
