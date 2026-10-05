import argparse
import os
import sys
from pathlib import Path
from PySide6.QtWidgets import QApplication
from .codex.adapter import CodexAdapter
from .ui.window import MainWindow


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--home',type=Path)
    parser.add_argument('--state-dir',type=Path,default=Path(os.environ.get('LOCALAPPDATA',str(Path.home())))/'ProjectConversationManager')
    parser.add_argument('--screenshot',type=Path)
    args=parser.parse_args()
    if args.home is None:
        import json
        settings=args.state_dir/'settings.json'
        try: saved=json.loads(settings.read_text(encoding='utf-8')).get('home') if settings.exists() else None
        except (ValueError,OSError): saved=None
        args.home=Path(saved or os.environ.get('CODEX_HOME',str(Path.home()/'.codex')))
    app=QApplication(sys.argv[:1]);app.setApplicationName('ProjectConversationManager')
    window=MainWindow(CodexAdapter(args.home),args.state_dir,auto_refresh=not args.screenshot)
    window.show()
    if args.screenshot:
        window.refresh(window.adapter.snapshot());window.project_list.setCurrentRow(0)
        from PySide6.QtCore import QTimer
        def capture():
            args.screenshot.parent.mkdir(parents=True,exist_ok=True);window.grab().save(str(args.screenshot));app.quit()
        QTimer.singleShot(300,capture)
    return app.exec()


if __name__=='__main__': raise SystemExit(main())
