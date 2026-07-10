# remaining

[English](README.md) | 한국어

AI 서비스의 남은 사용량을 터미널과 GNOME 상단 바에서 확인하는 도구입니다.

## 어떤 도구인가요?

`remaining`은 Codex, Z.ai, Ollama Cloud 같은 서비스에서 사용량과 초기화 시간을 가져옵니다. 같은 데이터를 다음 세 가지 방식으로 확인할 수 있습니다.

- 터미널 요약 화면
- 스크립트와 다른 프로그램에서 사용할 수 있는 JSON
- Ubuntu GNOME 상단 바의 팝업

## 지원 서비스

| 서비스 | 확인하는 곳 | 가져오는 정보 |
| --- | --- | --- |
| Codex | ChatGPT 사용량 API | 5시간, 주간, 모델별 한도, 리셋 크레딧 |
| Z.ai | Z.ai quota API | 5시간, 주간, 월간 도구 |
| Ollama Cloud | Ollama 웹 세션 | 5시간 또는 시간 단위 세션, 주간 |
| OpenCode Go | Workspace API 또는 로컬 기록 | 추정 세션, 주간, 월간 |
| MiniMax | Token/Coding Plan API | 5시간, 주간 |

Codex, Z.ai, Ollama Cloud가 기본으로 켜져 있습니다. 로그인 정보가 없거나 서비스가 응답을 거부하면 해당 서비스는 `auth_required`로 표시됩니다. 목록에 있다고 해서 해당 서비스를 구독하고 있다고 판단하지는 않습니다.

## 설치하기

### CLI

Rust 1.86 이상이 필요합니다.

```sh
git clone https://github.com/jsut21/remaining.git
cd remaining
cargo install --locked --path .
remaining
```

### GNOME

Ubuntu 24.04와 GNOME Shell 46에서 사용할 수 있습니다. CLI를 먼저 설치한 뒤 확장을 설치합니다.

```sh
./desktop/gnome/install.sh
```

설치하거나 업데이트한 뒤에는 로그아웃 후 다시 로그인해야 새 확장 코드가 적용됩니다. 제거할 때는 다음 명령을 사용합니다.

```sh
./desktop/gnome/uninstall.sh
```

확장 설정까지 지우려면 `--purge-settings`를 추가합니다. 이 명령은 CLI를 삭제하지 않습니다.

![GNOME 시연 화면](docs/media/remaining-gnome-demo.gif)

[원본 MP4 시연 영상 보기](docs/media/remaining-gnome-demo.mp4)

## 터미널에서 사용하기

```sh
remaining
remaining --json
remaining --json --pretty
remaining --provider codex,zai,ollama
remaining --all
remaining --list-providers
remaining --watch 300
```

브라우저 쿠키는 기본적으로 사용합니다. Ollama Cloud처럼 웹 세션이 필요한 서비스를 조회하지 않으려면 명시적으로 끌 수 있습니다.

```sh
remaining --no-browser-cookies
```

사용자 지정 endpoint는 공식 HTTPS 도메인으로 제한되어 있습니다. 목적지를 신뢰하는 경우에만 예외를 허용합니다.

```sh
remaining --provider zai --allow-custom-endpoints
```

전체 옵션은 `remaining --help`에서 확인할 수 있습니다.

## 인증 정보와 개인정보

- 선택한 서비스만 조회합니다.
- 브라우저 쿠키는 지원하는 서비스의 도메인에서만 사용합니다.
- 환경 변수와 서비스별 로컬 인증 파일을 사용할 수 있습니다.
- 터미널 및 JSON 출력에 인증 정보를 넣지 않습니다.
- 일반 HTTP와 인증 정보가 URL에 포함된 endpoint는 거부합니다.
- 브라우저 쿠키 사용을 막으려면 `--no-browser-cookies`를 사용합니다.

추가 서비스를 켜기 전에 [provider별 인증 방식](docs/providers.md)을 확인하십시오. 버그를 제보할 때 인증 파일이나 가공하지 않은 민감한 출력을 첨부하지 마십시오.

## 알아둘 점

서비스의 대시보드와 값이 잠시 다를 수 있습니다. 캐시, 초기화 시점, 서비스 API 변경이 원인일 수 있습니다.

사용량을 확인할 수 없는 경우 임의로 100% 남았다고 표시하지 않고 `null`로 반환합니다. JSON 결과의 형식은 [schemas/remaining.schema.json](schemas/remaining.schema.json)에 정의되어 있습니다.

이 프로젝트는 아직 초기 단계이며, OpenAI, Z.ai, Ollama, OpenCode 또는 MiniMax와 관련이 없는 비공식 도구입니다.

구현과 provider 관련 문서는 [docs](docs/)에 있습니다.

## 라이선스

MIT 라이선스입니다. 자세한 내용은 [LICENSE](LICENSE)를 확인하십시오.
