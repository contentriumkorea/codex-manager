"""Readable operation history backed by the same journal used for recovery."""
from pathlib import Path
from datetime import datetime, timezone
from PySide6.QtCore import Qt, QUrl
from PySide6.QtGui import QDesktopServices
from PySide6.QtWidgets import QDialog, QVBoxLayout, QHBoxLayout, QLabel, QLineEdit, QTreeWidget, QTreeWidgetItem, QPlainTextEdit, QPushButton, QSplitter, QWidget, QInputDialog

LABELS = {'move': '프로젝트 옮기기', 'merge': '프로젝트 합치기', 'connections': '프로젝트 이름·연결 변경', 'rename-thread': '대화 제목 변경', 'rename-threads': '대화 제목 일괄 변경', 'management-delete': '프로젝트·대화 삭제', 'file-edit': '파일 작업', 'import': '백업 복원', 'export-cleanup': '백업 후 원본 정리'}
STATES = {'completed': '완료', 'cancelled': '취소', 'failed': '실패', 'needs_recovery': '복구 확인 필요', 'not_started': '미실행', 'interrupted': '중단됨'}


def recovery_available(operation, journal):
    if operation['payload'].get('kind') == 'batch-summary': return False
    return any(r['id'] == operation['id'] for r in journal.pending() + journal.restorable())


def resources(operation):
    p = operation['payload']
    return set(p.get('resources', ())) | {item['project_id'] for item in p.get('items', ())} | ({p['target_id']} if p.get('target_id') else set())


def related_pending(operation, journal):
    p = operation['payload']; home = Path(p.get('home', ''))
    return [r for r in journal.pending() if Path(r['payload'].get('home', '')) == home and (r['id'] == operation['id'] or resources(r) & resources(operation))]


def retry_candidates(operation, journal):
    payload = operation['payload']
    if payload.get('kind') != 'batch-summary' or payload.get('action') not in ('move', 'merge', 'backup'): return ()
    if any(e['payload'].get('retry_report') for e in journal.events(operation['id'])) or related_pending(operation, journal): return ()
    return tuple(item['project_id'] for item in payload.get('items', ()) if item['status'] != 'completed')


def operation_label(operation):
    p = operation['payload']; title = p.get('label') or LABELS.get(p.get('kind'), '작업')
    source = p.get('source') or p.get('project') or {}
    if isinstance(source, dict) and source.get('name'): title += ' · ' + source['name']
    return title


def operation_items(operation):
    p = operation['payload']
    if p.get('kind') == 'batch-summary': return p.get('items', [])
    state = operation['state']; items = []
    for plan in p.get('plans', []):
        items.append({'name': Path(plan['source']).name, 'sources': [plan['source']], 'destinations': [plan.get('target') or plan.get('hold') or ''], 'status': state})
    if p.get('destinations'):
        for source, dest in p['destinations'].items():
            items.append({'name': Path(source).name, 'sources': [source], 'destinations': [str(dest)], 'status': state})
    if not items:
        changes = p.get('changes', [])
        for change in changes:
            items.append({'name': change.get('old_name', change.get('id', '대화')), 'sources': [], 'destinations': [], 'status': state})
    return items


class ActivityDialog(QDialog):
    def __init__(self, w):
        super().__init__(w); self.w = w; self.records = {}; self.current = None; self.home = w.adapter.home
        self.setWindowTitle('작업 내역 · 복구'); self.resize(1080, 670); self.setMinimumSize(790, 520)
        layout = QVBoxLayout(self)
        self.summary = QLabel('최근 작업의 결과와 변경된 위치를 확인하세요.'); self.summary.setWordWrap(True); layout.addWidget(self.summary)
        row = QHBoxLayout(); self.search = QLineEdit(); self.search.setPlaceholderText('작업·프로젝트·경로 검색'); self.search.setClearButtonEnabled(True)
        self.search.textChanged.connect(lambda: self.refresh()); row.addWidget(self.search)
        refresh = QPushButton('새로고침'); refresh.clicked.connect(lambda: self.refresh()); row.addWidget(refresh); layout.addLayout(row)
        split = QSplitter(Qt.Vertical); layout.addWidget(split, 1)
        self.list = QTreeWidget(); self.list.setRootIsDecorated(False); self.list.setHeaderLabels(['작업', '결과', '시각'])
        self.list.setColumnWidth(0, 450); self.list.setColumnWidth(1, 170); self.list.currentItemChanged.connect(self.select)
        split.addWidget(self.list)
        details = QWidget(); detail_layout = QVBoxLayout(details); detail_layout.setContentsMargins(0, 0, 0, 0)
        self.items = QTreeWidget(); self.items.setRootIsDecorated(False); self.items.setHeaderLabels(['항목', '결과', '이전 위치', '이후 / 예정 위치'])
        self.items.setColumnWidth(0, 170); self.items.setColumnWidth(1, 115); self.items.setColumnWidth(2, 285)
        self.items.currentItemChanged.connect(self.sync_open); detail_layout.addWidget(self.items, 2)
        self.detail = QPlainTextEdit(); self.detail.setReadOnly(True); detail_layout.addWidget(self.detail, 1); split.addWidget(details); split.setSizes([250, 320])
        buttons = QHBoxLayout()
        self.recover = QPushButton('선택한 작업 복구'); self.recover.clicked.connect(self.recover_selected); buttons.addWidget(self.recover)
        self.retry = QPushButton('미완료 항목 다시 시도'); self.retry.clicked.connect(self.retry_selected); buttons.addWidget(self.retry)
        self.open_folder = QPushButton('변경된 폴더 열기'); self.open_folder.clicked.connect(self.open_selected_folder); buttons.addWidget(self.open_folder)
        buttons.addStretch(); close = QPushButton('닫기'); close.clicked.connect(self.close); buttons.addWidget(close); layout.addLayout(buttons)
        self.refresh()

    def refresh(self, selected=None):
        if self.home != self.w.adapter.home:
            self.home = self.w.adapter.home; self.current = None
            self.summary.setText('현재 저장소의 작업 내역입니다.'); self.search.blockSignals(True); self.search.clear(); self.search.blockSignals(False)
        selected = selected or (self.current['id'] if self.current else None); query = self.search.text().casefold()
        self.list.blockSignals(True); self.list.clear(); self.records = {}
        recoverable = {r['id'] for r in self.w.recovery_items()}
        for operation in self.w.journal.history(self.w.adapter.home):
            p = operation['payload']; title = operation_label(operation)
            if query and query not in (title + ' ' + str(p)).casefold(): continue
            events = self.w.journal.events(operation['id']); recovered = any(e['payload'].get('recovered') for e in events)
            state = p.get('result_state', operation['state'])
            status = '복구 완료' if recovered else STATES.get(state, '진행 중 · 복구 가능')
            if operation['id'] in recoverable and state == 'completed': status += ' · 되돌리기 가능'
            if any(e['payload'].get('retry_report') for e in events): status += ' · 재시도 기록 있음'
            stamp = operation.get('created_at') or ''
            if stamp:
                try: stamp = datetime.fromisoformat(stamp).replace(tzinfo=timezone.utc).astimezone().strftime('%m-%d %H:%M')
                except ValueError: pass
            item = QTreeWidgetItem([title, status, stamp]); item.setData(0, Qt.UserRole, operation['id']); item.setToolTip(0, title)
            self.list.addTopLevelItem(item); self.records[operation['id']] = operation
            if operation['id'] == selected: self.list.setCurrentItem(item)
        if not self.list.currentItem() and self.list.topLevelItemCount(): self.list.setCurrentItem(self.list.topLevelItem(0))
        self.list.blockSignals(False); self.select(self.list.currentItem())

    def show_result(self, result):
        self.summary.setText('\n'.join([result.file_status, result.codex_status + ' · ' + result.cleanup_status, *result.errors]))
        self.search.clear(); self.refresh(result.report_id)
        if not result.report_id:
            self.list.clearSelection(); self.list.setCurrentItem(None); self.select(None)
        self.show(); self.raise_()

    def select(self, item, _previous=None):
        self.current = self.records.get(item.data(0, Qt.UserRole)) if item else None
        self.items.clear(); self.detail.clear(); self.recover.setEnabled(False); self.retry.setEnabled(False); self.open_folder.setEnabled(False)
        if not self.current: return
        op = self.current; p = op['payload']; events = self.w.journal.events(op['id'])
        texts = [operation_label(op)]
        if p.get('target_name'): texts.append('합친 뒤 프로젝트 이름: ' + p['target_name'])
        for data in operation_items(op):
            before = '\n'.join(data.get('sources', [])); after = '\n'.join(data.get('destinations', []))
            row = QTreeWidgetItem([data.get('name', ''), STATES.get(data.get('status'), '확인 필요'), before, after])
            row.setData(0, Qt.UserRole, data)
            for col in range(4): row.setToolTip(col, row.text(col))
            self.items.addTopLevelItem(row)
            if data.get('errors'): texts.append(data.get('name', '') + ': ' + '\n'.join(data['errors']))
        texts.extend(e['payload']['error'] for e in events if e['payload'].get('error'))
        available = recovery_available(op, self.w.journal) or bool(related_pending(op, self.w.journal))
        busy = self.w.shortcuts.busy()
        self.recover.setEnabled(available and not busy); self.retry.setEnabled(bool(retry_candidates(op, self.w.journal)) and not busy)
        if available: texts.append('복구 사본과 이후 변경을 확인한 뒤 복구합니다. 완료한 다른 작업은 유지됩니다.')
        elif p.get('kind') in ('move', 'merge') or p.get('action') in ('move', 'merge'):
            texts.append('완료한 이동·합치기는 자동 되돌리기를 지원하지 않습니다. 현재 프로젝트에서 새 이동 작업을 시작할 수 있습니다.')
        if p.get('kind') == 'batch-summary': texts.append('미실행 항목은 변경하지 않았습니다. 재시도 시 현재 프로젝트와 새 저장 위치를 다시 확인합니다.')
        if p.get('action') == 'backup': texts.append('검증이 끝난 백업만 백업 목록에 등록합니다. 실패한 위치의 일부 파일은 유지되며, 재시도는 새 위치에 저장합니다.')
        if any(e['payload'].get('retry_report') for e in events): texts.append('이 내역은 이미 재시도했습니다. 가장 최근 작업 내역에서 결과를 확인하세요.')
        for change in p.get('changes', []): texts.append(str(change.get('old_name', '')) + ' → ' + str(change.get('new_name', '')))
        if p.get('old_name') is not None: texts.append(str(p['old_name']) + ' → ' + str(p.get('new_name', '')))
        self.detail.setPlainText('\n\n'.join(texts))
        if self.items.topLevelItemCount(): self.items.setCurrentItem(self.items.topLevelItem(0))

    def sync_open(self, *_):
        item = self.items.currentItem(); data = item.data(0, Qt.UserRole) if item else {}
        self.open_folder.setEnabled(bool(data and data.get('status') == 'completed' and any(Path(p).exists() for p in data.get('destinations', []) if p)))

    def open_selected_folder(self):
        item = self.items.currentItem()
        if not item: return
        data = item.data(0, Qt.UserRole)
        for value in data.get('destinations', []):
            if not value: continue
            path = Path(value)
            if path.exists(): QDesktopServices.openUrl(QUrl.fromLocalFile(str(path if path.is_dir() else path.parent))); break

    def recover_selected(self):
        if not self.current or self.w.shortcuts.busy(): return
        operations = related_pending(self.current, self.w.journal)
        if recovery_available(self.current, self.w.journal) and not operations: operations = [self.current]
        if not operations: self.refresh(); return
        op = operations[0]
        if len(operations) > 1:
            labels = [operation_label(r) + ' · ' + r['id'][:8] for r in operations]
            label, ok = QInputDialog.getItem(self, '복구할 작업', '중단된 작업을 선택하세요.', labels, 0, False)
            if not ok: return
            op = operations[labels.index(label)]
        self.w.show_recovery(op['id'])

    def retry_selected(self):
        if self.current: self.w.retry_activity(self.current['id'])
