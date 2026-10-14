"""Throttled worker progress and monotonic, recent-sample transfer estimates."""
from collections import deque
from dataclasses import dataclass, replace
from time import monotonic


@dataclass(frozen=True)
class ProgressState:
    phase: str = '준비 중'
    activity: str = ''
    detail: str = ''
    current: int = 0
    total: int | None = None
    unit: str = 'bytes'
    transfer: bool = False
    item_index: int = 0
    item_total: int = 0
    item_name: str = ''
    segment: int = 0
    segment_started: float = 0
    segment_current: int = 0
    timestamp: float = 0


class ProgressReporter:
    """Also accepts the existing (current, total) callback protocol."""
    def __init__(self, emit, clock=monotonic, interval=.1):
        self.emit = emit
        self.clock = clock
        self.interval = interval
        self.state = ProgressState()
        self.last_emit = float('-inf')

    def _reset(self, **changes):
        now = self.clock()
        self.state = replace(self.state, segment=self.state.segment + 1,
                             segment_started=now, segment_current=changes.get('current', self.state.current),
                             **changes)

    def phase(self, title, total=None, unit='bytes', current=0, transfer=False):
        self._reset(phase=title, activity='', detail='', current=current, total=total,
                    unit=unit, transfer=transfer)
        self.flush(False)

    def activity(self, title, detail='', transfer=None):
        changes = dict(activity=title, detail=str(detail))
        if transfer is not None:
            changes['transfer'] = transfer
        self.state = replace(self.state, **changes)
        self.flush(False)

    def item(self, index, total, name):
        self._reset(item_index=index, item_total=total, item_name=name, phase='준비 중',
                    current=0, total=None, activity='', detail='', transfer=False)
        self.flush(False)

    def __call__(self, current, total):
        current = max(0, int(current))
        total = max(0, int(total)) if total is not None else None
        if total is not None:
            current = min(current, total)
        if current < self.state.current or total != self.state.total:
            self._reset(current=current, total=total)
        else:
            self.state = replace(self.state, current=current)
        self.flush(False)

    def flush(self, force=True):
        now = self.clock()
        # The UI also polls this immutable snapshot while an I/O call is blocked.
        self.state = replace(self.state, timestamp=now)
        if force or now - self.last_emit >= self.interval:
            self.last_emit = now
            self.emit(self.state)


class ProgressMetrics:
    def __init__(self, clock=monotonic):
        self.clock = clock
        self.started = clock()
        self.state = ProgressState()
        self.samples = deque()

    def update(self, state):
        if state.segment != self.state.segment or not self.samples:
            self.samples.clear()
            self.samples.append((state.segment_started, state.segment_current))
        self.state = state
        if state.current != self.samples[-1][1]:
            self.samples.append((state.timestamp, state.current))
        while len(self.samples) > 2 and self.samples[1][0] < state.timestamp - 5:
            self.samples.popleft()

    def elapsed(self):
        return max(0, self.clock() - self.started)

    def speed(self):
        if not self.state.transfer or not self.samples:
            return None
        now = self.clock()
        if now - self.samples[-1][0] >= 3:
            return 0.0
        while len(self.samples) > 2 and self.samples[1][0] <= now - 5:
            self.samples.popleft()
        elapsed = min(5, now - self.samples[0][0])
        if elapsed < .5 or len(self.samples) < 2:
            return None
        return max(0, (self.state.current - self.samples[0][1]) / elapsed)

    def remaining(self):
        speed = self.speed()
        if not speed or not self.state.total:
            return None
        return max(0, self.state.total - self.state.current) / speed


def phase(progress, title, **kwargs):
    if hasattr(progress, 'phase'):
        progress.phase(title, **kwargs)


def activity(progress, title, detail='', **kwargs):
    if hasattr(progress, 'activity'):
        progress.activity(title, detail, **kwargs)


def item(progress, index, total, name):
    if hasattr(progress, 'item'):
        progress.item(index, total, name)
