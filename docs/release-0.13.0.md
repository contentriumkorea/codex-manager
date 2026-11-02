# Codex Manager 0.13.0

**[Windows 설치 프로그램](https://github.com/contentriumkorea/codex-manager/releases/download/installer-v1/Codex-Manager-Setup.exe)** · **[포터블 ZIP](https://github.com/contentriumkorea/codex-manager/releases/latest/download/Codex-Manager-Windows-x64.zip)**

프로젝트 폴더에 작업 파일과 대화를 함께 보관하고, 다른 컴퓨터에서 그 폴더를 열어 작업을 이어갈 수 있도록 구성했습니다.

- 프로젝트별 **대화 자동 보관**을 켜면 Manager 실행 중 변경된 대화를 프로젝트 폴더 안에 저장합니다. 마지막 성공 시각과 최근 실패 이유를 확인하고 **지금 보관**으로 직접 저장할 수 있습니다.
- 일시적인 실패는 대기 간격을 늘리며 다시 시도합니다. 중단된 추가 폴더 사본은 내용과 소유권을 확인한 항목만 정리하며, 변경되었거나 확인할 수 없는 항목은 경고와 함께 보존합니다.
- 일반 대화·보관 대화·내부 작업 기록과 압축 대화를 검증합니다. 검증을 마친 뒤 보관본을 교체하며, 실패하거나 취소하면 이전 정상 보관본을 유지합니다.
- 주 작업 폴더의 일반 파일은 원래 위치를 사용합니다. 추가로 연결한 작업 폴더는 검증된 사본으로 함께 담고, 복원할 때 일반 작업 폴더로 연결합니다.
- **폴더 열기**에서 프로젝트 폴더나 보관 상위 폴더를 선택하고 새 프로젝트로 복원하거나 기존 프로젝트에 합칠 수 있습니다. 같은 ID의 대화는 내용과 연결을 확인하며, 충돌하는 기록을 덮어쓰지 않습니다.
- 기본 화면은 탐색기 한 개입니다. 우측 상단 분할 아이콘을 누르면 목적지 폴더를 선택하는 탐색기가 열립니다. 프로젝트와 대화 관리는 주 화면에서 처리합니다.

자동 보관은 Manager가 실행 중일 때 작동합니다. 프로젝트 연결을 바꾸거나 복원할 때는 Codex 앱과 관련 CLI 작업을 종료하세요. 원본 수리·삭제와 독립 첨부 파일의 자동 수집은 수행하지 않습니다. 손상된 기록, 없는 폴더, 해결되지 않은 외부 참조는 완료로 표시하지 않습니다.

기존 경량 설치파일을 계속 사용할 수 있습니다. **프로그램 포터블 ZIP**은 실행용이며, 작업을 이전할 때는 대화가 보관된 프로젝트 폴더를 함께 복사하세요.

[사용 안내](https://github.com/contentriumkorea/codex-manager/blob/main/docs/user-guide.md) · [호환성과 검증 범위](https://github.com/contentriumkorea/codex-manager/blob/main/docs/compatibility.md)
