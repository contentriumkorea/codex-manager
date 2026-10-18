<p align="center">
  <img src="docs/images/cover.svg" alt="Codex Manager — 프로젝트와 대화를 함께 정리하고 옮기는 Windows 앱" width="100%">
</p>

<h1 align="center">Codex Manager</h1>

<p align="center">
  <strong>프로젝트·대화·작업 파일을 한곳에서 관리하세요.</strong><br>
  Codex 작업을 외장하드에 함께 보관하고, 다른 컴퓨터에서 원하는 프로젝트로 가져오는 Windows 앱입니다.
</p>

<p align="center">
  <a href="https://github.com/contentriumkorea/codex-manager/releases/latest"><img src="docs/images/badge-release.svg" alt="Release 0.10.4"></a>
  <a href="#적용-범위와-제한"><img src="docs/images/badge-windows.svg" alt="Windows x64 지원"></a>
  <a href="#적용-범위와-제한"><img src="docs/images/badge-license.svg" alt="라이선스 미지정"></a>
</p>

<p align="center">
  <a href="#빠르게-시작하기">설치하기</a> ·
  <a href="https://github.com/contentriumkorea/codex-manager/releases/download/installer-v1/Codex-Manager-Setup.exe">다운로드</a> ·
  <a href="#사용-모습">사용 모습</a> ·
  <a href="docs/user-guide.md">상세 안내</a>
</p>

---

## 빠르게 시작하기

**Windows x64 · 인터넷 연결 필요.** 약 2.5 MB 설치파일이 최신 앱을 내려받습니다. 받은 설치파일은 다음 버전에도 그대로 사용할 수 있습니다.

<p align="center">
  <a href="https://github.com/contentriumkorea/codex-manager/releases/download/installer-v1/Codex-Manager-Setup.exe"><img src="docs/images/download-install.svg" alt="Windows 설치형 다운로드 — 약 2.5 MB" width="238"></a>
  <a href="https://github.com/contentriumkorea/codex-manager/releases/latest/download/Codex-Manager-Windows-x64.zip"><img src="docs/images/download-portable.svg" alt="설치 없이 실행하는 포터블 ZIP 다운로드" width="238"></a>
</p>

설치파일 실행 → 설치 폴더 선택 → 시작 메뉴에서 **Codex Manager** 실행.

[상세 설치 및 업데이트 안내 →](docs/user-guide.md)

## 작업을 정리하고, 함께 옮기세요

| 01 · 탐색기처럼 정리 | 02 · 프로젝트와 대화 관리 | 03 · 다른 컴퓨터로 이전 |
| :--- | :--- | :--- |
| **익숙한 조작으로 빠르게** | **연결 관계를 한곳에서** | **대화와 파일을 함께** |
| 여러 항목 선택, 끌어 놓기, 열 제목 정렬, F2 이름 변경과 Delete 삭제. | 이름 변경, 대화 이동, 프로젝트 합치기, 폴더 연결 관리와 복구 가능한 삭제. | 외장하드로 보내고 백업 상위 폴더에서 가져오기. 전송 속도·경과 시간·예상 남은 시간 표시. |

## 사용 모습

<p align="center">
  <img src="docs/images/manager.png" alt="예시 프로젝트로 촬영한 Codex Manager 실제 앱 화면" width="880">
</p>

**프로젝트를 선택하면 연결된 폴더와 대화를 함께 확인할 수 있습니다.**

위 화면은 예시 프로젝트를 사용한 실제 앱 화면입니다. 화면의 버전 표시는 촬영 당시 기준이며, 최신 버전은 릴리스에서 확인할 수 있습니다.

<details>
<summary><strong>추가 예시 보기</strong></summary>

- 프로젝트나 대화를 대상 프로젝트로 끌어 놓아 합치거나 옮깁니다. **대상 프로젝트 이름을 유지**합니다.
- 프로젝트 표시 이름은 F2로 바꿉니다. 실제 작업 폴더 이름은 유지합니다.
- A 백업을 B 프로젝트로 가져오면 대화는 B에 연결되고, 파일은 B 아래 별도 A 폴더에 저장됩니다. 가져오기 전에 저장 위치를 변경할 수 있습니다.

[화면별 사용법과 단축키 →](docs/user-guide.md)

</details>

## 설치형과 포터블

| 방식 | 사용 방법 |
| :--- | :--- |
| **설치형 · 처음 사용하는 경우** | 작은 설치파일이 최신 앱을 내려받습니다. 시작 메뉴에서 실행하고 Windows 설정에서 제거할 수 있습니다. |
| **포터블 · 설치 없이 실행** | ZIP을 새 폴더에 풀고 `CodexManager/CodexManager.exe`를 실행합니다. 실행 파일과 `_internal` 폴더를 함께 보관하세요. |
| **앱 업데이트** | 시작할 때 업데이트를 확인하고 미리 준비합니다. **지금 업데이트**를 누르면 설치 후 다시 실행합니다. 왼쪽 **업데이트**에서도 확인할 수 있습니다. |

설치형과 포터블 모두 Manager 설정·작업 내역은 `%LOCALAPPDATA%/ProjectConversationManager`에 저장합니다. Codex 대화는 선택한 Codex 데이터 위치에, 작업 파일은 프로젝트 폴더에 있습니다. 작업을 이전할 때는 아래 **보내기 / 가져오기**를 사용하세요.

## 이렇게 사용하세요

```text
원래 컴퓨터: 프로젝트 선택 → 보내기… → 외장하드 폴더 선택
다른 컴퓨터: Codex와 Manager 준비 → 가져오기… → 백업 상위 폴더 선택
가져올 항목 선택 → 새 프로젝트로 가져오기 또는 기존 프로젝트에 합치기
저장 위치 확인 → 가져오기 → 파일과 대화 확인
```

원래 컴퓨터와 다른 저장 경로를 선택해도 됩니다. 원본과 외장하드 백업은 자동으로 삭제하지 않습니다. [이전 후 원본 정리와 삭제 방법](docs/user-guide.md)을 확인하세요.

## 적용 범위와 제한

- 로컬 Windows x64의 Codex 프로젝트와 대화를 관리합니다. 새 컴퓨터의 Codex 로그인은 별도로 진행합니다.
- 프로젝트 연결 변경·가져오기·삭제는 Codex 앱을 종료한 뒤 진행하세요. 조회와 보내기는 Codex 실행 중에도 사용할 수 있습니다.
- 프로그램 ZIP에 사용자 대화·작업 파일·Manager 설정은 포함되지 않습니다. 대화와 파일을 함께 옮길 때는 **보내기 / 가져오기**를 사용하세요.
- 실제 계정의 두 번째 물리 PC 이어쓰기와 모바일 표시는 별도 검수가 필요합니다. [호환성 및 검증 범위](docs/compatibility.md)를 확인하세요.
- 이 저장소에는 별도의 오픈소스 라이선스가 지정되어 있지 않습니다.

<details>
<summary><strong>상세 안내와 검증 범위</strong></summary>

파일과 대화의 이동·합치기·삭제는 격리된 테스트 저장소에서 검증합니다. 배포 과정에서는 설치·업데이트·제거, 포터블 실행, 공개 다운로드의 해시와 버전을 확인합니다.

[사용 안내](docs/user-guide.md) · [호환성](docs/compatibility.md) · [개발·배포 안내](docs/releasing.md) · [0.10.4 배포 점검 범위](docs/release-audit-0.10.4.md) · [문제 신고](https://github.com/contentriumkorea/codex-manager/issues)

</details>

<details>
<summary><strong>English overview</strong></summary>

Codex Manager is an independent Windows x64 application for organizing local Codex projects, conversations, and working folders. Rename, move, merge, and export projects with their conversations and files, then import them into a new or existing project on another PC.

The small online installer downloads the latest application. A portable ZIP is also available. Use **Send / Import** to transfer your work; the application ZIP does not contain your conversations or settings. Close Codex before changing project connections, importing, or deleting. Cross-device continuation on a second physical PC and mobile display require separate validation.

</details>

---

<p align="center">
  <strong>CONTENTRIUM · 콘텐츠리움</strong><br>
  라이선스 미지정 ·
  <a href="https://github.com/contentriumkorea/codex-manager/releases/latest">Latest release</a>
</p>

<sub>Codex Manager는 로컬 Codex용 독립 도구이며 OpenAI 공식 제품은 아닙니다.</sub>

<p align="center">
  <a href="https://contentrium.com/"><img src="docs/images/contentrium-home.svg" alt="콘텐츠리움 홈페이지 바로가기" width="260" height="52"></a>
</p>
