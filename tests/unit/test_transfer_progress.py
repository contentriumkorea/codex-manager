import pytest


def test_recent_speed_eta_and_stalled_transfer():
    from project_manager.progress import ProgressReporter, ProgressMetrics
    now = [0.0]
    events = []
    reporter = ProgressReporter(events.append, clock=lambda: now[0], interval=0)
    metrics = ProgressMetrics(clock=lambda: now[0])
    reporter.phase('파일 복사', total=1000, transfer=True)
    metrics.update(events[-1])
    now[0] = 2.0
    reporter(200, 1000)
    metrics.update(events[-1])
    assert metrics.speed() == pytest.approx(100)
    assert metrics.remaining() == pytest.approx(8)
    now[0] = 4.0
    reporter(600, 1000)
    metrics.update(events[-1])
    assert metrics.speed() == pytest.approx(150)
    assert metrics.remaining() == pytest.approx(400 / 150)
    now[0] = 10.0
    assert metrics.speed() == 0
    assert metrics.remaining() is None
    assert metrics.elapsed() == 10


def test_phase_and_batch_reset_eta_but_keep_total_elapsed():
    from project_manager.progress import ProgressReporter, ProgressMetrics
    now = [0.0]
    metrics = ProgressMetrics(clock=lambda: now[0])
    reporter = ProgressReporter(metrics.update, clock=lambda: now[0], interval=0)
    reporter.item(1, 2, 'A')
    reporter.phase('파일 복사', total=1000, transfer=True)
    now[0] = 2.0
    reporter(500, 1000)
    assert metrics.remaining() == pytest.approx(2)
    reporter.activity('복사본 검증', 'example.bin', transfer=False)
    assert metrics.speed() is None and metrics.remaining() is None
    now[0] = 8.0
    reporter.item(2, 2, 'B')
    reporter.phase('폴더 검사')
    assert metrics.state.item_index == 2
    assert metrics.state.item_name == 'B'
    assert metrics.state.total is None
    assert metrics.remaining() is None and metrics.elapsed() == 8
    reporter.phase('파일 복사', total=0, transfer=True)
    assert metrics.remaining() is None


def test_speed_window_does_not_include_a_long_previous_stall():
    from project_manager.progress import ProgressReporter, ProgressMetrics
    now = [0.0]
    metrics = ProgressMetrics(clock=lambda: now[0])
    reporter = ProgressReporter(metrics.update, clock=lambda: now[0], interval=0)
    reporter.phase('파일 복사', total=10000, transfer=True)
    for stamp, value in [(1, 100), (2, 200), (100, 300), (101, 400), (102, 500)]:
        now[0] = stamp
        reporter(value, 10000)
    # Only the 300 bytes observed in seconds 97..102 belong to this estimate.
    assert metrics.speed() == pytest.approx(60)
    assert metrics.remaining() == pytest.approx(9500 / 60)


def test_progress_events_are_bounded_for_many_tiny_files():
    from project_manager.progress import ProgressReporter
    now = [0.0]
    events = []
    reporter = ProgressReporter(events.append, clock=lambda: now[0])
    reporter.phase('파일 복사', total=10000, transfer=True)
    for i in range(10000):
        now[0] += .0001
        reporter.activity('복사 중', f'{i}.txt', transfer=True)
        reporter(i + 1, 10000)
    reporter.flush()
    assert len(events) <= 15
    assert events[-1].current == 10000 and events[-1].detail == '9999.txt'


class Probe:
    def __init__(self):
        self.phases = []
        self.values = []
        self.activities = []
    def phase(self, title, **kwargs):
        self.phases.append(title)
    def activity(self, title, detail='', **kwargs):
        self.activities.append((title, str(detail)))
    def __call__(self, current, total):
        self.values.append((current, total))


def test_verified_copy_reports_chunks_and_distinguishes_verification(tmp_path):
    from project_manager.files import scan_roots, copy_verified
    source = tmp_path / 'source'
    source.mkdir()
    data = b'a' * (3 * 1024 * 1024)
    (source / 'large.bin').write_bytes(data)
    inventory = scan_roots({'r': source}, True)
    probe = Probe()
    result = copy_verified(inventory, {'r': source}, {'r': tmp_path / 'dest'}, probe, lambda: False)
    assert result.ok
    assert (tmp_path / 'dest' / 'large.bin').read_bytes() == data
    assert any(0 < current < total for current, total in probe.values)
    assert probe.phases, 'Copy must name its phase before sending byte counts'
    assert any('검증' in title for title, _ in probe.activities), '100% copied is not yet verified'


def test_file_manager_copy_reports_before_finishing(tmp_path):
    from project_manager.file_edits import copy_item, signature
    source = tmp_path / 'large.bin'
    data = b'b' * (3 * 1024 * 1024)
    source.write_bytes(data)
    destination = tmp_path / 'copy.bin'
    probe = Probe()
    copy_item(source, destination, signature(source), progress=probe)
    assert destination.read_bytes() == data
    assert any(0 < done < total for done, total in probe.values)
    assert any('검증' in title for title in probe.phases)


def test_scan_can_cancel_inside_a_large_file_without_writing(tmp_path):
    from project_manager.files import scan_roots
    source = tmp_path / 'source'
    source.mkdir()
    (source / 'large.bin').write_bytes(b'x' * (3 * 1024 * 1024))
    values = []
    with pytest.raises(InterruptedError):
        scan_roots({'r': source}, True, progress=lambda done, total: values.append(done),
                   cancelled=lambda: bool(values))
    assert values and values[0] == 1024 * 1024
    assert (source / 'large.bin').stat().st_size == 3 * 1024 * 1024
