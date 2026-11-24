// 공유 서버(Firebase) 설정. 비워 두면 이 PC의 실시간감시 데이터만 보여준다.
// apiKey 는 공개용 값이라 웹에 그대로 두어도 된다 (쓰기 권한은 database.rules.json 이 막는다).
window.DHO_CONFIG = {
  apiKey: "AIzaSyA_AjVrTcsBaQu6235rCyXSgXa-vBkJRUY",
  databaseURL: "https://real-time-dho-default-rtdb.asia-southeast1.firebasedatabase.app",
  // 왼쪽 아래 서버 선택 메뉴에 항상 보일 서버 (데이터가 들어온 다른 서버도 자동으로 추가됨)
  servers: ["헬레네", "폴라리스", "에이레네"],
  // 크로스 월드가 없는 단독 서버: 등록자(캐릭터)를 읽지 않는다. 실시간감시는 이 서버를 '원래 서버' 후보에서 뺀다
  solo: ["에이레네"],
  // 게임 서버 접속 주소 → 서버 (실시간감시가 지금 들어가 있는 서버를 아는 데 씀. 2026-11-11 확인)
  // 실시간감시 최신 버전: 지도를 연 PC의 실시간감시가 이보다 낮으면(또는 버전을 안 보내는 옛 exe면) "최신버전이 아닙니다. (최신버전 3.1)"
  latest: { "감지기": "3.45", "에이레네": "3.45" },   // 2026-11-21 사용자: 3.45 릴리즈 (트레이 일반모드 1초 10번 / 렉감소모드 1초 4번, /me ver "3.45") → 아니면 "최신버전으로 업데이트해주세요"
  // 2026-11-21 사용자: 구글 드라이브 배포 중단 → download(드라이브 폴더) 뺌. 받기는 GitHub 릴리즈만
  // GitHub 릴리즈 바로 받기 (2026-11-20 사용자: 크롬·구글 드라이브 없이도). latest = 늘 마지막 릴리즈라 새 버전도 이 주소 그대로.
  // 파일 이름은 영어여야 함 (GitHub가 한글 이름을 default.exe로 바꿈). 릴리즈 페이지에는 '헬폴 감지기.exe' 같은 한글 이름표가 보임
  release: {
    page: "https://github.com/nakhunn-png/dho-realtime-price/releases/latest",
    헬폴: "https://github.com/nakhunn-png/dho-realtime-price/releases/latest/download/HelPol-Gamjigi.exe",
    에이레네: "https://github.com/nakhunn-png/dho-realtime-price/releases/latest/download/Eirene-Gamjigi.exe",
  },
  // 지도 왼쪽 아래 초록 불 옆 "건의사항 / 오류신고 (관리자 오픈카톡)" 링크
  kakao: "https://open.kakao.com/o/sX4V90Uc",
  addrs: { "203.242.208.218:9067": "헬레네", "203.242.208.228:9066": "폴라리스" },
  // 게임 서버 IP 대역 → 서버 (같은 서버도 접속할 때·해역을 옮길 때 IP와 포트가 바뀐다. 2026-11-12 로그로 확인한 대역)
  ranges: { "203.242.208.216-219": "헬레네", "203.242.208.226-229": "폴라리스" },
};
