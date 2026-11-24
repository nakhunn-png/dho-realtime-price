# 시세 모듈 sidecar 소스 (price-side.exe)

DHO Navigator 모듈 "타이거냥이 시세공유 및 남만 모듈"(com.tigernyang.realtime-price)에 들어 있는
`runtime/sidecars/price/price-side.exe`의 소스입니다. 라이선스 MIT.

## 하는 일
- Navigator가 연결한 게임 창만 Windows Graphics Capture로 캡처해 서버·도시 이름, 교역소 매각 창(교역품·시세 %), 남만 재고를 읽습니다.
- 읽은 시세와 등록자(캐릭터) 이름을 실시간 시세 지도의 Firebase Realtime Database에 올립니다 (`web/config.js`의 주소, 익명 로그인).
- 게임 화면 위쪽 가운데에 알림 띠를 그립니다.
- 게임 메모리 읽기나 키보드·마우스 입력 자동화는 하지 않습니다.

## 폴더
- `rust/modside` — sidecar 본체 (Navigator와 ndjson-v1로 통신)
- `rust/crates/recog` — 화면 글자·숫자 인식
- `rust/crates/capture` — 게임 창 캡처
- `rust/crates/netaddr` — 접속한 게임 서버 주소로 서버 판별
- `rust/crates/gamji/src/{config,cloud}.rs` — 설정 읽기, Firebase 업로드
- `web/config.js`, `web/data/*.json` — 빌드할 때 exe 안에 들어가는 공개 설정·품목·도시 목록

## 빌드 (Windows x86_64, Rust 2024 edition)
```
cd rust/modside
cargo build --release
```
결과: `target/release/price-side.exe`