"""One review screen between choosing an external folder and importing it."""
from pathlib import Path
from PySide6.QtCore import Qt
from PySide6.QtWidgets import (QDialog,QVBoxLayout,QHBoxLayout,QLabel,QTreeWidget,QTreeWidgetItem,
    QComboBox,QLineEdit,QPushButton,QCheckBox,QDialogButtonBox,QFileDialog,QPlainTextEdit)
from ..migration import plan_import
from ..settings import read_settings


class ImportDialog(QDialog):
    def __init__(self,w,discovery):
        super().__init__(w);self.w=w;self.discovery=discovery;self.snapshot=w.snapshot;self._plans=()
        self.setWindowTitle('다른 컴퓨터의 작업 가져오기');self.resize(810,650);self.setMinimumSize(680,530)
        layout=QVBoxLayout(self)
        intro=QLabel('가져올 프로젝트를 선택하세요. 파일과 대화를 함께 가져옵니다.');intro.setWordWrap(True);layout.addWidget(intro)
        self.list=QTreeWidget();self.list.setRootIsDecorated(False);self.list.setHeaderLabels(['가져올 프로젝트','대화','파일 용량']);self.list.setColumnWidth(0,410)
        for i,bundle in enumerate(discovery.bundles):
            m=bundle.manifest;size=sum(e['size'] for e in m['inventory'])
            from .window import bytes_text
            item=QTreeWidgetItem([m['project']['name'],str(len(m['conversations'])),bytes_text(size)])
            item.setData(0,Qt.UserRole,i);item.setCheckState(0,Qt.Checked);item.setToolTip(0,str(bundle.path));self.list.addTopLevelItem(item)
        layout.addWidget(self.list,1)
        if discovery.errors:
            issues=QPlainTextEdit();issues.setReadOnly(True);issues.setMaximumHeight(80)
            issues.setPlainText('읽을 수 없는 백업 또는 검색하지 못한 위치\n'+'\n'.join(discovery.errors));layout.addWidget(issues)
        row=QHBoxLayout();row.addWidget(QLabel('가져오는 방식'));self.target=QComboBox();self.target.addItem('각각 새 프로젝트로 가져오기',None)
        for p in self.snapshot.projects:self.target.addItem('기존 프로젝트에 합치기 · '+p.name,p.id)
        row.addWidget(self.target,1);layout.addLayout(row)
        row=QHBoxLayout();row.addWidget(QLabel('파일 저장 위치'));self.destination=QLineEdit()
        saved=read_settings(w.state_dir/'settings.json').get('import_directory')
        default=Path(saved) if saved else self.snapshot.projects[0].roots[0].parent if self.snapshot.projects and self.snapshot.projects[0].roots else Path.home()/'Documents'/'Codex Projects'
        self.destination.setText(str(default));row.addWidget(self.destination,1)
        browse=QPushButton('변경…');browse.clicked.connect(self.browse);row.addWidget(browse);layout.addLayout(row)
        self.skip=QCheckBox('이미 있는 대화는 이 컴퓨터의 기록을 유지하고 건너뛰기');self.skip.setChecked(True);layout.addWidget(self.skip)
        self.preview=QPlainTextEdit();self.preview.setReadOnly(True);self.preview.setMaximumHeight(155);layout.addWidget(self.preview)
        self.status=QLabel();self.status.setWordWrap(True);layout.addWidget(self.status)
        self.buttons=QDialogButtonBox(QDialogButtonBox.Ok|QDialogButtonBox.Cancel);self.buttons.button(QDialogButtonBox.Ok).setText('가져오기 시작');self.buttons.button(QDialogButtonBox.Cancel).setText('취소')
        self.buttons.accepted.connect(self.accept);self.buttons.rejected.connect(self.reject);layout.addWidget(self.buttons)
        self.list.itemChanged.connect(self.sync);self.destination.textChanged.connect(self.sync);self.target.currentIndexChanged.connect(self.target_changed);self.skip.toggled.connect(self.sync)
        self.sync()

    def browse(self):
        path=QFileDialog.getExistingDirectory(self,'가져온 작업 파일을 저장할 폴더',self.destination.text())
        if path:self.destination.setText(path)

    def target_changed(self):
        project=next((p for p in self.snapshot.projects if p.id==self.target.currentData()),None)
        if project and project.roots:self.destination.setText(str(project.roots[0]))
        self.sync()

    def sync(self,*_):
        selected=[self.discovery.bundles[self.list.topLevelItem(i).data(0,Qt.UserRole)] for i in range(self.list.topLevelItemCount()) if self.list.topLevelItem(i).checkState(0)==Qt.Checked]
        self._plans=();errors=[]
        try:
            if not self.destination.text().strip() or not Path(self.destination.text()).is_absolute():raise ValueError('파일을 저장할 절대 경로를 선택하세요.')
            self._plans=plan_import(selected,self.snapshot,Path(self.destination.text()),self.target.currentData())
            if any(b.manifest['dependencies'] for b in selected):errors.append('외부 첨부·작업 폴더가 빠진 백업은 가져올 수 없습니다. 해당 항목을 선택 해제하고 원래 컴퓨터에서 연결 범위를 확인하세요.')
        except (ValueError,OSError) as exc:errors.append(str(exc))
        duplicates=sum(len(p.duplicates) for p in self._plans);self.skip.setVisible(bool(duplicates))
        if duplicates and not self.skip.isChecked():errors.append('같은 대화가 이미 있습니다. 기존 기록 유지에 체크하거나 다른 Codex 데이터 위치를 선택하세요.')
        self.preview.setPlainText('\n\n'.join(p.bundle.manifest['project']['name']+' → '+('새 프로젝트' if p.target_id is None else self.target.currentText().split(' · ',1)[-1])+'\n'+'\n'.join(map(str,p.destinations.values())) for p in self._plans))
        threads=sum(len(p.bundle.manifest['conversations'])-len(p.duplicates) for p in self._plans)
        summary=f'프로젝트 {len(self._plans)}개 · 새 대화 {threads}개'+(f' · 중복 대화 {duplicates}개는 기존 기록 유지' if duplicates else '')
        if self.target.currentData():summary+='\n대상 프로젝트 이름을 유지하며, 파일은 위에 표시된 별도 폴더에 저장합니다.'
        summary+='\n외장하드의 백업은 그대로 남습니다. 가져오기 전에 전체 파일을 검증합니다.'
        if self.discovery.errors:summary+=f'\n읽을 수 없는 백업 또는 검색하지 못한 위치 {len(self.discovery.errors)}개는 위에 표시했습니다.'
        self.status.setToolTip('\n'.join(self.discovery.errors));self.status.setText('\n'.join(errors) if errors else summary)
        self.buttons.button(QDialogButtonBox.Ok).setEnabled(bool(self._plans) and not errors)

    def plans(self):return self._plans
