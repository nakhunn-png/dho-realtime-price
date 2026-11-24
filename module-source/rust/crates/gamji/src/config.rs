//! 실행 파일 안에 넣은 web/config.js · 품목·도시 이름 (Python 판도 exe 안의 web 폴더를 읽는다)
use serde_json::Value;

static CONFIG_JS: &str = include_str!("../../../../web/config.js");
static GOODS_JSON: &str = include_str!("../../../../web/data/goods.json");
static CITIES_JSON: &str = include_str!("../../../../web/data/cities.json");

/// Python `cloud.read_config` 가 읽는 값
#[derive(Debug, Default)]
pub struct Config {
    pub api_key: String,
    pub database_url: String,
    pub servers: Vec<String>,
    /// 크로스 월드가 없는 단독 서버 (에이레네)
    pub solo: Vec<String>,
    /// 게임 서버 접속 주소 → 서버
    pub addrs: Vec<(String, String)>,
    /// 게임 서버 IP 대역 → 서버 ("1.2.3.10-19": "헬레네")
    pub ranges: Vec<(String, String)>,
}

/// `key\s*:\s*<open>...<close>` 의 안쪽 (처음 맞는 곳)
fn block<'a>(txt: &'a str, key: &str, open: char, close: char) -> Option<&'a str> {
    let mut from = 0;
    while let Some(p) = txt[from..].find(key) {
        let rest = txt[from + p + key.len()..].trim_start();
        if let Some(r) = rest.strip_prefix(':') {
            let r = r.trim_start();
            if let Some(r) = r.strip_prefix(open) {
                if let Some(e) = r.find(close) {
                    return Some(&r[..e]);
                }
            }
        }
        from += p + key.len();
    }
    None
}

/// "..." 로 싼 글자들
fn quoted(s: &str) -> Vec<String> {
    s.split('"').skip(1).step_by(2).map(String::from).collect()
}

/// `"키" : "값"` 짝 (키는 `ok` 글자로만 된 것)
fn pairs(s: &str, ok: impl Fn(char) -> bool) -> Vec<(String, String)> {
    let mut out = vec![];
    // 따옴표 사이에 ':' 만 있는 곳이 짝이다
    let parts: Vec<&str> = s.split('"').collect();
    let mut i = 1;
    while i + 2 < parts.len() {
        let (k, mid, v) = (parts[i], parts[i + 1], parts[i + 2]);
        if mid.trim() == ":" && !k.is_empty() && k.chars().all(&ok) && !v.is_empty() {
            out.push((k.to_string(), v.to_string()));
            i += 4;
        } else {
            i += 2;
        }
    }
    out
}

pub fn read_config() -> Option<Config> {
    let txt = CONFIG_JS;
    let s = |k: &str| block(txt, k, '"', '"').map(|v| v.trim().to_string()).unwrap_or_default();
    let cfg = Config {
        api_key: s("apiKey"),
        database_url: s("databaseURL"),
        servers: block(txt, "servers", '[', ']').map(quoted).unwrap_or_default(),
        solo: block(txt, "solo", '[', ']').map(quoted).unwrap_or_default(),
        addrs: block(txt, "addrs", '{', '}').map(|b| pairs(b, |c| c.is_ascii_digit() || c == '.' || c == ':')).unwrap_or_default(),
        ranges: block(txt, "ranges", '{', '}').map(|b| pairs(b, |c| c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or_default(),
    };
    (!cfg.api_key.is_empty() && !cfg.database_url.is_empty()).then_some(cfg)
}

fn names(txt: &str, key: &str) -> Vec<String> {
    let v: Value = serde_json::from_str(txt).expect("이름 목록");
    v[key].as_array().map(|a| a.iter().filter_map(|g| g["name"].as_str().map(String::from)).collect()).unwrap_or_default()
}

/// (품목 이름, 도시 이름)
pub fn load_names() -> (Vec<String>, Vec<String>) {
    (names(GOODS_JSON, "goods"), names(CITIES_JSON, "cities"))
}
