"""One progress window for transfer, backup and other background jobs."""
import math
from time import monotonic
from PySide6.QtCore import Qt, QTimer, Signal
from PySide6.QtWidgets import QDialog, QVBoxLayout, QHBoxLayout, QLabel, QProgressBar, QPushButton
from ..progress import ProgressMetrics


def bytes_text(value):
    for unit in ('B', 'KB', 'MB', 'GB', 'TB'):
        if value < 1024 or unit == 'TB':
            return f'{value:.1f} {unit}'
        value /= 1024


def duration(seconds):
    seconds = max(0, math.ceil(seconds))
    hours, seconds = divmod(seconds, 3600)
    minutes, seconds = divmod(seconds, 60)
    if hours:
        return f'{hours}시간 {minutes:02d}분 {seconds:02d}초'
    return f'{minutes}분 {seconds:02d}초' if minutes else f'{seconds}초'


class JobProgressDialog(QDialog):
    cancel_requested = Signal()

    def __init__(self, title, parent=None, clock=monotonic):
        super().__init__(parent)
        self.setWindowTitle(title)
        self.setWindowModality(Qt.WindowModal)
        self.setMinimumWidth(520)
        self.resize(580, 350)
        self.metrics = ProgressMetrics(clock)
        self.active = True
        self.cancelling = False
        layout = QVBoxLayout(self)
        layout.setContentsMargins(24, 24, 24, 20)
        layout.setSpacing(12)
        heading = QLabel(title)
        heading.setObjectName('detailTitle')
        layout.addWidget(heading)
        self.item_label = QLabel()
        self.item_label.setTextFormat(Qt.PlainText)
        self.item_label.setObjectName('muted')
        layout.addWidget(self.item_label)
        self.phase_label = QLabel('준비 중')
        self.phase_label.setTextFormat(Qt.PlainText)
        layout.addWidget(self.phase_label)
        self.bar = QProgressBar()
        self.bar.setRange(0, 0)
        layout.addWidget(self.bar)
        self.amount_label = QLabel('용량 확인 중…')
        self.amount_label.setObjectName('muted')
        layout.addWidget(self.amount_label)
        self.file_label = QLabel(' ')
        self.file_label.setTextFormat(Qt.PlainText)
        self.file_label.setObjectName('muted')
        layout.addWidget(self.file_label)
        stats = QHBoxLayout()
        for caption, attr in [('전송 속도', 'speed_label'), ('총 경과 시간', 'elapsed_label'), ('예상 남은 시간', 'remaining_label')]:
            column = QVBoxLayout()
            label = QLabel(caption)
            label.setObjectName('muted')
            column.addWidget(label)
            value = QLabel('—')
            column.addWidget(value)
            setattr(self, attr, value)
            stats.addLayout(column, 1)
        layout.addLayout(stats)
        self.note_label = QLabel('남은 시간은 현재 복사 단계 기준입니다.')
        self.note_label.setObjectName('muted')
        layout.addWidget(self.note_label)
        buttons = QHBoxLayout()
        buttons.addStretch()
        self.cancel_button = QPushButton('취소')
        self.cancel_button.clicked.connect(self.request_cancel)
        buttons.addWidget(self.cancel_button)
        layout.addLayout(buttons)
        self.timer = QTimer(self)
        self.timer.setInterval(250)
        self.timer.timeout.connect(self.render)
        self.timer.start()
        self.render()

    def update_progress(self, state):
        self.metrics.update(state)
        self.render()

    def render(self):
        state = self.metrics.state
        width = max(200, self.width() - 48)
        self.item_label.setText(self.fontMetrics().elidedText(
            f'{state.item_index} / {state.item_total}  ·  {state.item_name}' if state.item_total else '', Qt.ElideMiddle, width))
        self.item_label.setToolTip(state.item_name)
        self.item_label.setVisible(bool(state.item_total))
        self.phase_label.setText(state.phase + (' · ' + state.activity if state.activity else ''))
        self.file_label.setText(self.fontMetrics().elidedText(state.detail, Qt.ElideMiddle, width) or ' ')
        self.file_label.setToolTip(state.detail)
        if state.total:
            self.bar.setRange(0, 1000)
            self.bar.setValue(int(state.current / state.total * 1000))
            self.bar.setFormat('현재 단계 %p%')
            amount = f'{bytes_text(state.current)} / {bytes_text(state.total)}' if state.unit == 'bytes' else f'{state.current} / {state.total}개'
        else:
            self.bar.setRange(0, 0)
            amount = (f'{bytes_text(state.current)} 처리 · 전체 용량 확인 중…' if state.current else '용량 확인 중…') if state.total is None and state.unit == 'bytes' else '처리 중…'
        if state.total and state.current == state.total and not state.transfer and state.activity:
            self.bar.setRange(0, 0)
        self.amount_label.setText(amount)
        speed = self.metrics.speed()
        self.speed_label.setText(bytes_text(speed) + '/s' if speed is not None and state.unit == 'bytes' else '—')
        self.elapsed_label.setText(duration(self.metrics.elapsed()))
        remaining = self.metrics.remaining()
        self.remaining_label.setText(('약 ' + duration(remaining)) if remaining is not None else ('계산 중…' if state.transfer else '—'))
        self.note_label.setText('남은 시간은 현재 복사 단계 기준입니다. 검증·대화 연결 시간은 별도입니다.' if state.transfer else '현재 작업을 처리하고 있습니다. 경과 시간은 계속 표시됩니다.')
        if self.cancelling:
            self.remaining_label.setText('—')
            self.note_label.setText('취소 요청을 처리 중입니다. 현재 단계가 안전하게 끝날 때까지 기다려 주세요.')

    def request_cancel(self):
        if not self.active or self.cancelling:
            return
        self.cancelling = True
        self.cancel_button.setEnabled(False)
        self.cancel_button.setText('취소 중…')
        self.cancel_requested.emit()
        self.render()

    def reject(self):
        if self.active:
            self.request_cancel()
        else:
            super().reject()

    def closeEvent(self, event):
        if self.active:
            self.request_cancel()
            event.ignore()
        else:
            event.accept()

    def complete(self):
        self.active = False
        self.timer.stop()
        self.close()
