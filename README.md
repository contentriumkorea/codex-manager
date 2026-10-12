# Codex Manager

## [⬇ Windows 설치 프로그램 다운로드](https://github.com/contentriumkorea/codex-manager/releases/latest/download/Codex-Manager-Setup.exe)

**위 링크를 누르면 설치 파일을 바로 받습니다.** 받은 `Codex-Manager-Setup.exe`를 실행한 뒤 시작 메뉴에서 **Codex Manager**를 검색하세요. Python 설치나 압축 해제가 필요 없습니다. 설치할 드라이브와 폴더도 선택할 수 있습니다.

Codex의 프로젝트·대화·작업 파일을 함께 정리하고, 외장하드에 보관하거나 다른 컴퓨터로 가져오는 Windows 앱입니다.

![Codex Manager 화면](docs/images/manager.png)

### 다른 컴퓨터로 옮기기

1. **원래 컴퓨터:** 프로젝트 선택 → **보내기…** → 외장하드 폴더 선택.
2. **다른 컴퓨터:** Codex와 Codex Manager 설치 → **가져오기…** → 백업이 들어 있는 상위 폴더 선택.
3. 자동으로 찾은 프로젝트를 선택하고 **새 프로젝트로 가져오기** 또는 **기존 프로젝트에 합치기**를 고릅니다. 저장 위치를 확인하고 시작하세요.

파일과 대화를 함께 옮깁니다. 기존 프로젝트에 합치면 그 프로젝트의 이름을 유지합니다. 파일은 별도 폴더에 들어가며, 같은 대화가 이미 있으면 기존 기록을 유지하도록 선택할 수 있습니다. 원래 컴퓨터와 외장하드의 내용은 자동으로 삭제하지 않습니다.

### 삭제와 정리

- **삭제… / Delete:** 복구 가능 삭제 또는 완전 삭제를 선택합니다. 작업 파일 삭제는 별도 체크입니다.
- **복구 가능 삭제:** 이 컴퓨터에 복구 사본을 자동 저장합니다. 보관 위치를 매번 고를 필요가 없습니다.
- **완전 삭제:** 복구 사본과 휴지통 없이 삭제합니다. 확인 체크 후 실행하며 되돌릴 수 없습니다.
- **이름 변경 / F2**, 여러 항목 선택, 끌어 놓기, 열 제목 클릭 정렬을 지원합니다. 추가 관리 기능은 **더 보기**와 우클릭 메뉴에 있습니다.

프로젝트 연결 변경·가져오기·삭제는 Codex 앱을 종료한 뒤 진행합니다. 조회와 보내기는 Codex 실행 중에도 사용할 수 있습니다.

### 업데이트

시작할 때 새 버전을 확인하고 미리 다운로드·검증합니다. **지금 업데이트**를 누르면 준비된 버전을 설치하고 다시 실행합니다. 왼쪽 **업데이트**에서도 확인할 수 있습니다. 설정, 대화, 작업 파일과 백업은 유지합니다.

[사용 안내](docs/user-guide.md) · [변경 내용](https://github.com/contentriumkorea/codex-manager/releases/latest) · [문제 신고](https://github.com/contentriumkorea/codex-manager/issues)

<details>
<summary>설치 없이 사용하는 포터블 버전</summary>

[포터블 ZIP 다운로드](https://github.com/contentriumkorea/codex-manager/releases/latest/download/Codex-Manager-Windows-x64.zip) → 압축 해제 → `CodexManager/CodexManager.exe` 실행. 실행 파일과 `_internal`을 포함한 폴더 전체를 함께 보관하세요. 처음 사용하는 경우 위의 설치 프로그램을 권장합니다.

</details>

<details>
<summary>개발과 검증</summary>

```powershell
.venv/Scripts/python.exe -m pytest -q
.venv/Scripts/python.exe -m PyInstaller --noconfirm --clean packaging/project-manager.spec
.venv/Scripts/python.exe tools/build_release.py
.venv/Scripts/python.exe tools/build_setup.py --compiler "C:/Program Files (x86)/Inno Setup 6/ISCC.exe" --dist dist/CodexManager --output releases/v0.10.0
.venv/Scripts/python.exe tools/verify_update.py --release releases/v0.10.0 --root .test-artifacts/update-verification
.venv/Scripts/python.exe tools/acceptance.py --isolated-root .test-artifacts/new-check --report .test-artifacts/report.json --fixture-only
```

정식 배포는 `vX.Y.Z` 태그에 설치 프로그램, 포터블 ZIP, `update.json`, `SHA256SUMS.txt`를 함께 게시합니다. 앱 업데이트는 ZIP을 사용합니다. Windows 설치 프로그램은 [Inno Setup](https://jrsoftware.org/)으로 만듭니다.

[호환성 및 검증 범위](docs/compatibility.md). 실제 계정 이어쓰기·두 번째 물리 PC·모바일은 별도 검수가 필요합니다. 자동 검증은 격리 저장소를 사용하며 사용자 원본을 삭제하지 않습니다.

</details>

로컬 Codex용 독립 도구이며 OpenAI 공식 제품은 아닙니다.
