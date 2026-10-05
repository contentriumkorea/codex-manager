# Codex Manager

Windows용 로컬 Codex 프로젝트·대화·폴더 관리 도구. 다크 UI로 이동, 합치기, 연결 관리, 대화 포함 백업과 새 경로 복원을 제공합니다.

사용자는 [GitHub Releases](https://github.com/contentriumkorea/codex-manager/releases/latest)에서 Windows ZIP을 내려받아 압축을 풀고, `CodexManager` 폴더의 `CodexManager.exe`를 실행합니다. 폴더 전체를 보관하세요. [사용 안내](docs/user-guide.md)를 참고하세요.

시작할 때 최신 정식 버전을 확인합니다. 새 버전이 있으면 변경 내용을 보고 **업데이트 설치·재시작**을 누를 수 있습니다. 앱의 **업데이트** 메뉴에서도 수동 확인과 시작 시 확인 설정을 제공합니다. 다운로드와 파일별 SHA-256을 검증한 뒤 교체하고, 새 프로그램이 정상 시작하지 않으면 이전 프로그램으로 복구합니다.

프로젝트·대화·작업 파일·백업을 프로그램 배포에 넣지 않습니다. 기존 설정 위치 `%LOCALAPPDATA%/ProjectConversationManager`를 유지합니다. 로컬 Codex용 독립 도구이며 OpenAI 공식 제품은 아닙니다.

개발:

```powershell
.venv/Scripts/python.exe -m pytest tests -q
.venv/Scripts/python.exe -m PyInstaller --noconfirm --clean packaging/project-manager.spec
.venv/Scripts/python.exe tools/build_release.py
.venv/Scripts/python.exe tools/verify_update.py --release releases/v0.2.0 --root .test-artifacts/update-verification
.venv/Scripts/python.exe tools/acceptance.py --isolated-root .test-artifacts/new-check --report .test-artifacts/report.json --fixture-only
```

실제 계정 이어쓰기·두 번째 물리 PC·모바일은 외부 검수 대기입니다. `--fixture-only` 없이 검수 도구를 실행하면 해당 미실시를 보고하고 종료 코드 2를 반환합니다. 원본 데이터는 구현 검증에서 변경하지 않았습니다.

정식 배포는 `vX.Y.Z` 태그와 `Codex-Manager-Windows-x64.zip`, `update.json`, `SHA256SUMS.txt`를 함께 게시합니다. 버전을 올릴 때 `src/project_manager/version.py`와 `pyproject.toml`을 함께 수정하고 실행 파일을 새로 빌드하세요. 초안·사전 배포는 업데이트 채널에 포함하지 않습니다.
