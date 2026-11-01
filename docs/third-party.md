# 포함 구성요소와 개인정보 안내

Codex Manager에는 Microsoft Edge WebView2 Runtime이 포함됩니다. 처음 실행할 때 해당 구성요소의 Microsoft 이용약관을 확인하고 동의해야 합니다. 설정에서도 약관을 다시 확인할 수 있습니다.

Microsoft Defender SmartScreen은 기본으로 활성화되어 있습니다. 이 기능은 최종 사용자 정보를 수집하여 Microsoft에 전송합니다. 자세한 내용은 [Microsoft 개인정보처리방침](https://aka.ms/privacy)과 [Microsoft Edge 개인정보 백서의 SmartScreen 안내](https://learn.microsoft.com/en-us/microsoft-edge/privacy-whitepaper#smartscreen)를 확인하세요.

포함된 런타임에는 [Microsoft Edge WebView2 Runtime Fixed Version 이용약관](https://developer.microsoft.com/microsoft-edge/api/eula/webview2?locale=en-us&fixed=true)이 적용됩니다. 전체 약관은 앱과 `_internal/webview2-eula.html`에 포함되어 있습니다. AVC 관련 고지와 Microsoft 및 공급자에 적용되는 보호 조건도 이 약관에서 확인할 수 있습니다.

런타임의 모든 구성 파일과 고지는 유지됩니다. Chromium 및 관련 구성요소의 내장 고지는 `_internal/WebView2/show_third_party_software_licenses.bat`에서 볼 수 있습니다. 기타 포함 라이브러리의 고지는 `_internal/THIRD-PARTY-NOTICES.txt`에 있습니다. 이 런타임은 앱 업데이트에 포함하여 갱신합니다.

프로그램의 업데이트 확인은 콘텐츠리움 GitHub 공개 릴리스에 접속합니다. 프로젝트 백업은 사용자가 지정한 저장장치에 기록하며, 이 백업 기능이 대화나 작업 파일을 GitHub에 업로드하지 않습니다. Codex와 Microsoft 구성요소의 자체 통신에는 각각 해당 서비스의 정책이 적용됩니다.
