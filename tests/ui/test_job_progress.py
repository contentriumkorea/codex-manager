from PySide6.QtCore import Qt


def test_elapsed_updates_without_byte_events_and_cancel_stays_visible(qtbot):
    from project_manager.ui.progress import JobProgressDialog
    now = [0.0]
    dialog = JobProgressDialog('프로젝트 백업', clock=lambda: now[0])
    qtbot.addWidget(dialog)
    dialog.show()
    now[0] = 65.0
    qtbot.waitUntil(lambda: '1분 05초' in dialog.elapsed_label.text(), timeout=1500)
    with qtbot.waitSignal(dialog.cancel_requested):
        qtbot.mouseClick(dialog.cancel_button, Qt.LeftButton)
    assert dialog.isVisible()
    assert not dialog.cancel_button.isEnabled()
    dialog.close()
    assert dialog.isVisible(), 'A running worker must not lose its progress window on close'
    dialog.complete()
    assert not dialog.isVisible()


def test_progress_shows_speed_and_clears_eta_during_verification(qtbot):
    from project_manager.progress import ProgressReporter
    from project_manager.ui.progress import JobProgressDialog
    now = [0.0]
    dialog = JobProgressDialog('백업', clock=lambda: now[0])
    qtbot.addWidget(dialog)
    reporter = ProgressReporter(dialog.update_progress, clock=lambda: now[0], interval=0)
    reporter.item(2, 5, '프로젝트 B')
    reporter.phase('파일 복사', total=10 * 1024 * 1024, transfer=True)
    now[0] = 2.0
    reporter(2 * 1024 * 1024, 10 * 1024 * 1024)
    assert '1.0 MB/s' in dialog.speed_label.text()
    assert '8초' in dialog.remaining_label.text()
    assert '2 / 5' in dialog.item_label.text()
    assert dialog.bar.value() == 200
    reporter.activity('복사본 검증', 'large.bin', transfer=False)
    assert '/s' not in dialog.speed_label.text()
    assert '8초' not in dialog.remaining_label.text()
    assert '검증' in dialog.phase_label.text()
    dialog.complete()


def test_window_job_delivers_worker_updates_and_waits_for_cancellation(qtbot, tmp_path):
    from project_manager.ui.window import MainWindow
    from project_manager.ui.progress import JobProgressDialog
    from project_manager.codex.adapter import CodexAdapter
    window = MainWindow(CodexAdapter(tmp_path / 'home', isolated=True), tmp_path / 'state', auto_refresh=False)
    qtbot.addWidget(window)
    results = []
    def copy_work(worker):
        worker.report.phase('파일 복사', total=100, transfer=True)
        worker.report.activity('복사 중', 'current.bin')
        worker.report(25, 100)
        worker.report.flush()
        worker.cancelled.wait(5)
        return 'cancelled' if worker.cancelled.is_set() else 'timeout'
    window.run_job('파일 이동', copy_work, results.append)
    dialog = window.findChild(JobProgressDialog)
    qtbot.waitUntil(lambda: dialog.bar.value() == 250)
    assert dialog.file_label.toolTip() == 'current.bin' and dialog.isVisible()
    qtbot.mouseClick(dialog.cancel_button, Qt.LeftButton)
    qtbot.waitUntil(lambda: not window.workers, timeout=5000)
    assert results == ['cancelled']


def test_verification_transition_appears_even_when_worker_is_blocked(qtbot, tmp_path):
    import threading
    from project_manager.ui.window import MainWindow
    from project_manager.ui.progress import JobProgressDialog
    from project_manager.codex.adapter import CodexAdapter
    window = MainWindow(CodexAdapter(tmp_path / 'home', isolated=True), tmp_path / 'state', auto_refresh=False)
    qtbot.addWidget(window)
    verify = threading.Event()
    release = threading.Event()
    def copy_work(worker):
        worker.report.interval = 60
        worker.report.phase('파일 복사', total=100, transfer=True)
        worker.report(100, 100)
        worker.report.flush()
        verify.wait(5)
        worker.report.activity('복사본 검증', 'large.bin', transfer=False)
        release.wait(5)  # Simulates a slow disk read with no more progress calls.
    window.run_job('백업', copy_work)
    dialog = window.findChild(JobProgressDialog)
    try:
        qtbot.waitUntil(lambda: dialog.bar.value() == 1000)
        verify.set()
        qtbot.waitUntil(lambda: '검증' in dialog.phase_label.text(), timeout=1500)
        assert dialog.bar.maximum() == 0
        assert '/s' not in dialog.speed_label.text()
    finally:
        verify.set();release.set()
        qtbot.waitUntil(lambda: not window.workers, timeout=5000)
