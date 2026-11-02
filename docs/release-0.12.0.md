# Codex Manager 0.12.0

**[Windows 설치 프로그램 · 약 2.5 MB](https://github.com/contentriumkorea/codex-manager/releases/download/installer-v1/Codex-Manager-Setup.exe)** · **[포터블 ZIP](https://github.com/contentriumkorea/codex-manager/releases/latest/download/Codex-Manager-Windows-x64.zip)**

양쪽 탐색기에서 서로 다른 Codex 저장소를 열고 프로젝트·대화·작업 파일을 함께 옮길 수 있습니다.

- 각 탐색기의 저장소, 프로젝트, 폴더, 검색, 정렬과 선택을 따로 유지합니다. 구분선으로 너비를 조절할 수 있습니다.
- 프로젝트·대화·파일을 여러 개 선택해 반대편으로 끌어 놓습니다. Ctrl을 누르면 복사를 선택합니다.
- 전송 전에 대상 부모 폴더와 이동·복사를 바꾸고, 실제 저장 위치를 미리 확인합니다. 기존 프로젝트에 넣으면 대상 이름을 유지합니다.
- 대화와 연결된 작업 파일을 함께 전송합니다. 다른 프로젝트·대화가 사용하는 원본 파일과 일부 대화만 옮긴 원본 작업 폴더는 유지하며 미리보기에 표시합니다.
- 공통 진행 표시에서 작업을 확인하고 취소할 수 있습니다. 중단된 전송은 작업 내역에서 이어서 진행합니다.
- Windows 경로의 슬래시 표기가 달라 폴더 이동 되돌리기를 잘못 거부하던 문제를 수정했습니다. 실제로 변경된 작업 폴더는 계속 보호합니다.

완료된 저장소 간 전송은 일반 되돌리기·Ctrl+Z 대상이 아닙니다. 이동한 원본 대화의 복구 사본은 대상 드라이브에 남을 수 있으며 작업 내역에서 위치와 크기를 확인합니다. 전송 중에는 임시 사본과 최종 파일을 함께 보관할 여유 공간이 필요합니다. 기존 파일을 덮어쓰지 않으며 대화 ID 충돌, 공유·보호 경로와 해결되지 않은 연결을 검사합니다.

기존 설정과 작업 내역을 유지합니다. 변경 작업 전에는 Codex 앱과 관련 CLI 작업을 종료하세요. 실제 계정의 두 번째 물리 PC 이어쓰기, 모바일 표시, 물리 마우스 드래그와 여러 DPI 조합은 별도 검수가 필요합니다.

[사용 안내](https://github.com/contentriumkorea/codex-manager/blob/main/docs/user-guide.md) · [호환성 및 제한](https://github.com/contentriumkorea/codex-manager/blob/main/docs/compatibility.md) · [구성요소 안내](https://github.com/contentriumkorea/codex-manager/blob/main/docs/third-party.md)
