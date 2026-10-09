# Codex Manager

파일 탐색기처럼 사용하는 Windows용 Codex 프로젝트·대화·폴더 관리 도구. 화이트·실버 글래스모피즘 UI로 이동, 합치기, 연결 관리, 대화 포함 백업과 새 경로 복원을 제공합니다. 블루·라벤더 배경과 반투명 패널, 선명한 차콜 글자를 사용하며 메뉴·확인창·업데이트 화면도 밝은 톤으로 통일했습니다.

프로젝트·대화 우클릭으로 이름 변경과 삭제를 처리합니다. 여러 대화를 선택해 다른 프로젝트로 이동하거나 삭제할 수 있습니다. 삭제 전 복구 사본을 검증하고, 작업 복구에서 되살립니다. 실제 작업 파일 삭제는 별도 선택이며 기본값은 파일 유지입니다.

전체 프로젝트 용량을 자동 계산해 큰 순으로 표시합니다. 명시적 소속이 없는 옛 대화는 폴더·부모 관계로 분류하고, 여러 Codex 저장소를 자동 검색·등록해 전환할 수 있습니다. 폴더 기준 분류는 원본 소속과 구분하고, 변경·정리 전에 연결을 확정합니다.

사용자는 [GitHub Releases](https://github.com/contentriumkorea/codex-manager/releases/latest)에서 Windows ZIP을 내려받아 압축을 풀고, `CodexManager` 폴더의 `CodexManager.exe`를 실행합니다. 폴더 전체를 보관하세요. [사용 안내](docs/user-guide.md)를 참고하세요.

시작할 때 최신 정식 버전을 확인하고 새 버전을 미리 다운로드·검증합니다. **지금 업데이트**를 누르면 바로 설치·재시작합니다. 다운로드 중에 누르면 진행률을 보여주며 준비되는 즉시 자동 설치합니다. 진행 중인 프로젝트 작업은 완료를 기다립니다. 앱의 **업데이트** 메뉴에서 수동 확인, 재시도, 준비 취소와 시작 시 확인 설정을 제공합니다. 설치 도우미의 준비 응답을 받은 뒤 앱을 닫고, 새 프로그램이 정상 시작하지 않으면 이전 프로그램으로 복구합니다.

프로젝트·대화·작업 파일·백업을 프로그램 배포에 넣지 않습니다. 기존 설정 위치 `%LOCALAPPDATA%/ProjectConversationManager`를 유지합니다. 로컬 Codex용 독립 도구이며 OpenAI 공식 제품은 아닙니다.

개발:

```powershell
.venv/Scripts/python.exe -m pytest tests -q
.venv/Scripts/python.exe -m PyInstaller --noconfirm --clean packaging/project-manager.spec
.venv/Scripts/python.exe tools/build_release.py
.venv/Scripts/python.exe tools/verify_update.py --release releases/v0.8.0 --root .test-artifacts/update-verification
.venv/Scripts/python.exe tools/acceptance.py --isolated-root .test-artifacts/new-check --report .test-artifacts/report.json --fixture-only
```

실제 계정 이어쓰기·두 번째 물리 PC·모바일은 외부 검수 대기입니다. `--fixture-only` 없이 검수 도구를 실행하면 해당 미실시를 보고하고 종료 코드 2를 반환합니다. 원본 데이터는 구현 검증에서 변경하지 않았습니다.

정식 배포는 `vX.Y.Z` 태그와 `Codex-Manager-Windows-x64.zip`, `update.json`, `SHA256SUMS.txt`를 함께 게시합니다. 버전을 올릴 때 `src/project_manager/version.py`와 `pyproject.toml`을 함께 수정하고 실행 파일을 새로 빌드하세요. 초안·사전 배포는 업데이트 채널에 포함하지 않습니다.
