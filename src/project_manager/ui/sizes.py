import threading
from PySide6.QtCore import QObject,QThread,Signal
from ..folder_sizes import measure_folders,FolderSize


class SizeWorker(QThread):
    measured=Signal(str,object)
    def __init__(self,projects):
        super().__init__();self.projects=projects;self.cancelled=threading.Event()
    def run(self):
        for p in self.projects:
            if self.cancelled.is_set():break
            try:result=measure_folders(p.roots,self.cancelled.is_set)
            except InterruptedError:break
            except Exception as exc:result=FolderSize(skipped=1,warnings=(str(exc),))
            if not self.cancelled.is_set():self.measured.emit(p.id,result)


class SizeScanner(QObject):
    changed=Signal(str);settled=Signal()
    def __init__(self,parent):
        super().__init__(parent);self.results={};self.roots={};self.pending={};self.worker=None;self.paused=False;self.generation=0
    def reset(self,projects):
        roots={p.id:p.roots for p in projects}
        self.results={pid:r for pid,r in self.results.items() if self.roots.get(pid)==roots.get(pid) and pid in roots}
        self.roots=roots;self.pending={p.id:p for p in projects};self.generation+=1
        if self.worker:self.worker.cancelled.set()
        elif not self.paused:self.resume()
    def stop(self):
        self.paused=True
        if self.worker:self.worker.cancelled.set();return False
        return True
    def resume(self):
        self.paused=False
        if self.worker or not self.pending:return
        worker=SizeWorker(list(self.pending.values()));generation=self.generation;self.worker=worker
        def result(pid,value):
            if generation!=self.generation or worker.cancelled.is_set():return
            self.results[pid]=value;self.pending.pop(pid,None);self.changed.emit(pid)
        def finish():
            self.worker=None;worker.deleteLater()
            if not self.paused and self.pending:self.resume()
            else:self.settled.emit()
        worker.measured.connect(result);worker.finished.connect(finish);worker.start()
