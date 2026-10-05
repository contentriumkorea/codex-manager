# Project Conversation Manager

Windows용 로컬 Codex 프로젝트·대화·폴더 관리 도구. 다크 UI로 이동, 합치기, 연결 관리, 대화 포함 백업과 새 경로 복원을 제공합니다.

사용자는 `ProjectManager` 폴더 전체를 복사하고 `ProjectManager.exe`를 실행합니다. 실행 파일만 따로 옮기지 마세요. [사용 안내](docs/user-guide.md)를 참고하세요.

개발:

```powershell
.venv/Scripts/python.exe -m pytest tests -q
.venv/Scripts/python.exe -m PyInstaller --noconfirm --clean packaging/project-manager.spec
.venv/Scripts/python.exe tools/acceptance.py --isolated-root .test-artifacts/new-check --report .test-artifacts/report.json --fixture-only
```

실제 계정 이어쓰기·두 번째 물리 PC·모바일은 외부 검수 대기입니다. `--fixture-only` 없이 검수 도구를 실행하면 해당 미실시를 보고하고 종료 코드 2를 반환합니다. 원본 데이터는 구현 검증에서 변경하지 않았습니다.
