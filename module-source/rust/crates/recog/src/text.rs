//! 글자 다루기: 교역소 주인 대사, 길드 개척도시, difflib 비슷한 글자 찾기 (Python 판과 같은 규칙)

/// difflib.SequenceMatcher(None, a, b).ratio() — 짧은 글자(200자 미만)라 autojunk 는 쓰이지 않는다
pub fn ratio(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let t = a.len() + b.len();
    if t == 0 {
        return 1.0;
    }
    2.0 * matches(&a, &b) as f64 / t as f64
}

/// get_matching_blocks 의 길이 합
fn matches(a: &[char], b: &[char]) -> usize {
    use std::collections::HashMap;
    let mut b2j: HashMap<char, Vec<usize>> = HashMap::new();
    for (j, &c) in b.iter().enumerate() {
        b2j.entry(c).or_default().push(j);
    }
    let longest = |alo: usize, ahi: usize, blo: usize, bhi: usize| -> (usize, usize, usize) {
        let (mut bi, mut bj, mut bk) = (alo, blo, 0usize);
        let mut j2len: HashMap<usize, usize> = HashMap::new();
        for i in alo..ahi {
            let mut new: HashMap<usize, usize> = HashMap::new();
            if let Some(js) = b2j.get(&a[i]) {
                for &j in js {
                    if j < blo {
                        continue;
                    }
                    if j >= bhi {
                        break;
                    }
                    let k = if j > 0 { j2len.get(&(j - 1)).copied().unwrap_or(0) } else { 0 } + 1;
                    new.insert(j, k);
                    if k > bk {
                        (bi, bj, bk) = (i + 1 - k, j + 1 - k, k);
                    }
                }
            }
            j2len = new;
        }
        (bi, bj, bk)
    };
    let mut total = 0;
    let mut q = vec![(0, a.len(), 0, b.len())];
    while let Some((alo, ahi, blo, bhi)) = q.pop() {
        let (i, j, k) = longest(alo, ahi, blo, bhi);
        if k > 0 {
            total += k;
            if alo < i && blo < j {
                q.push((alo, i, blo, j));
            }
            if i + k < ahi && j + k < bhi {
                q.push((i + k, ahi, j + k, bhi));
            }
        }
    }
    total
}

/// difflib.get_close_matches(word, possibilities, n=1, cutoff)
pub fn close_match<'a>(word: &str, possibilities: &'a [String], cutoff: f64) -> Option<&'a String> {
    let mut best: Option<(f64, &String)> = None;
    for x in possibilities {
        let r = ratio(x, word);
        if r >= cutoff && best.is_none_or(|(s, n)| r > s || (r == s && x > n)) {
            best = Some((r, x));
        }
    }
    best.map(|b| b.1)
}

/// 한글 음절을 자모(초성·중성·종성)로 푼다. 한글이 아닌 글자는 그대로
fn jamo(t: &str) -> String {
    let mut o = String::new();
    for c in t.chars() {
        if is_hangul(c) {
            let i = c as u32 - 0xAC00;
            o.push(char::from_u32(0x1100 + i / 588).unwrap());
            o.push(char::from_u32(0x1161 + i % 588 / 28).unwrap());
            if i % 28 != 0 {
                o.push(char::from_u32(0x11A7 + i % 28).unwrap());
            }
        } else {
            o.push(c);
        }
    }
    o
}

/// 글자 수가 같은 이름 중 자모가 거의 같은 것 (2026-11-21: 도시명 '캘리컷'을 OCR이 '캘리것'으로 읽음 — 음절로는 67%라 close_match가 놓친다).
/// 가장 가까운 것이 하나뿐일 때만 (둘이 똑같이 가까우면 어느 쪽인지 모르니 None)
pub fn close_match_jamo<'a>(word: &str, possibilities: &'a [String], cutoff: f64) -> Option<&'a String> {
    let (n, wj) = (word.chars().count(), jamo(word));
    let mut sc: Vec<(f64, &String)> =
        possibilities.iter().filter(|x| x.chars().count() == n).map(|x| (ratio(&jamo(x), &wj), x)).filter(|(r, _)| *r >= cutoff).collect();
    sc.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    match sc.as_slice() {
        [(_, x)] => Some(x),
        [(a, x), (b, _), ..] if a > b => Some(x),
        _ => None,
    }
}

pub fn is_hangul(c: char) -> bool {
    ('가'..='힣').contains(&c)
}

/// `[가-힣A-Za-z0-9]`
pub fn is_name_char(c: char) -> bool {
    is_hangul(c) || c.is_ascii_alphanumeric()
}

/// 길드 개척도시(길드명 + 개척도시)는 시세가 항상 100%라 수집 대상에서 뺀다
pub fn is_guild_city(name: &str) -> bool {
    match name.strip_suffix(" 개척도시") {
        Some(g) => (1..=16).contains(&g.chars().count()) && g.chars().all(is_name_char),
        None => false,
    }
}

/// OCR 도시 이름 `([가-힣A-Za-z0-9]{1,16})\s*개척도.{0,3}` 전체 일치 → 길드명
pub fn guild_city_prefix(raw: &str) -> Option<String> {
    let c: Vec<char> = raw.chars().collect();
    let key = ['개', '척', '도'];
    // 정규식처럼 길드명을 가장 길게 잡는 쪽(뒤쪽 '개척도')부터
    for p in (0..c.len().saturating_sub(2)).rev() {
        if c[p..p + 3] != key || c.len() - (p + 3) > 3 {
            continue;
        }
        let mut e = p;
        while e > 0 && c[e - 1].is_whitespace() {
            e -= 1;
        }
        let g = &c[..e];
        if (1..=16).contains(&g.len()) && g.iter().all(|&ch| is_name_char(ch)) {
            return Some(g.iter().collect());
        }
    }
    None
}

// ---------------------------------------------------------------- 교역소 주인 대사
const OWNER_LINES: [&str; 6] = [
    "어서 오세요",
    "지금 {k}{이가} 대폭락이야.. 휴",
    "지금 {k}{은는} 남아돌고 있어.",
    "지금 {k}{이가} 폭등하고 있어!",
    "지금 {k}{이가} 잘 팔려.",
    "경기가 좋아서 {k} 시세도 하늘 높은 줄을 모른다네!",
];
const GOODS_KINDS: [&str; 20] = [
    "식료품", "조미료", "가축", "의약품", "잡화", "주류", "염료", "광석", "공업품", "기호품", "섬유", "직물", "공예품", "미술품",
    "향신료", "귀금속", "향료", "보석", "무기류", "총포류",
];
/// 글자 인식이 자주 틀리는 말
const OCR_FIX: [(&str, &str); 4] = [("지급", "지금"), ("묻품", "물품"), ("남아돔", "남아돌"), ("부족햬", "부족해")];

/// 인정하는 주인 대사 (가나다 순)
pub fn owner_canon() -> Vec<String> {
    let mut out = Vec::new();
    for tpl in OWNER_LINES {
        let kinds: &[&str] = if tpl.contains("{k}") { &GOODS_KINDS } else { &[""] };
        for k in kinds {
            out.push(fill(tpl, k));
        }
    }
    out.sort();
    out.dedup();
    out
}

pub fn fix_ocr(t: &str) -> String {
    let mut t = t.to_string();
    for (a, b) in OCR_FIX {
        t = t.replace(a, b);
    }
    // `[|!](?=니다)` 지우기
    let c: Vec<char> = t.chars().collect();
    let mut out = String::new();
    for i in 0..c.len() {
        if (c[i] == '|' || c[i] == '!') && c.get(i + 1) == Some(&'니') && c.get(i + 2) == Some(&'다') {
            continue;
        }
        out.push(c[i]);
    }
    out
}

/// `[\s.!?…~•·‥]` 지우기 (말줄임표는 OCR에서 `•`·`·`로도 나온다)
fn strip_marks(t: &str) -> String {
    t.chars().filter(|&c| !(c.is_whitespace() || ".!?…~•·‥".contains(c))).collect()
}

/// 대사별 핵심어 (띄어쓰기·문장부호 뺀 글자). OCR이 문장 끝(`…휴` 등)을 망가뜨려도
/// 교역품 종류와 핵심어가 보이면 그 대사로 본다
const OWNER_KEYS: [(&str, &[&str]); 5] = [
    ("지금 {k}{이가} 대폭락이야.. 휴", &["대폭락", "폭락이"]),
    ("지금 {k}{은는} 남아돌고 있어.", &["남아돌", "아돌고"]),
    ("지금 {k}{이가} 폭등하고 있어!", &["폭등하"]),
    ("지금 {k}{이가} 잘 팔려.", &["잘팔려"]),
    ("경기가 좋아서 {k} 시세도 하늘 높은 줄을 모른다네!", &["하늘높은", "경기가좋아"]),
];

fn fill(tpl: &str, k: &str) -> String {
    let jong = k.chars().last().is_some_and(|c| (c as u32 - 0xAC00) % 28 != 0);
    tpl.replace("{k}", k).replace("{이가}", if jong { "이" } else { "가" }).replace("{은는}", if jong { "은" } else { "는" })
}

/// 교역품 종류 + 핵심어로 대사 찾기 (`key` 는 strip_marks 한 글자)
fn match_owner_keys(key: &str) -> Option<String> {
    let kind = GOODS_KINDS.iter().filter(|k| key.contains(*k)).max_by_key(|k| k.chars().count())?;
    let hits: Vec<&str> = OWNER_KEYS.iter().filter(|(_, ws)| ws.iter().any(|w| key.contains(w))).map(|(t, _)| *t).collect();
    // 핵심어가 둘 이상의 대사에 걸리면 어느 쪽인지 모르니 버린다
    if hits.len() == 1 { Some(fill(hits[0], kind)) } else { None }
}

/// OCR한 주인 대사를 인정 문장 중 가장 가까운 것으로 바꾼다. 비슷한 게 없으면 None
pub fn match_owner_line(t: &str) -> Option<String> {
    use std::sync::OnceLock;
    static CANON: OnceLock<Vec<(String, String)>> = OnceLock::new();
    let canon = CANON.get_or_init(|| owner_canon().into_iter().map(|c| (strip_marks(&c), c)).collect());
    let key = strip_marks(&fix_ocr(t));
    if key.is_empty() {
        return None;
    }
    let (mut best, mut score) = (None, 0.0);
    for (k, c) in canon {
        let r = ratio(&key, k);
        if r > score {
            (best, score) = (Some(c), r);
        }
    }
    if score >= 0.82 {
        return best.cloned();
    }
    // 2026-11-21 인사말 '어서 오세요'가 '어서 왜요'(사그레스)·'어처 오세요'(캘리컷)로 읽힘
    // → '어서'로 시작하거나 '오세요'로 끝나는 짧은 대사는 인사말 (다른 주인 대사는 모두 '지금'·'경기가'로 시작하고 길다)
    if (key.starts_with("어서") || key.ends_with("오세요")) && key.chars().count() <= 6 {
        return Some(OWNER_LINES[0].to_string());
    }
    match_owner_keys(&key)
}

/// 교역소주인·교역소의 도제 (OCR이 조금 틀려도, 띄어쓰기는 빼고 본다)
pub fn is_owner(speaker: &str) -> bool {
    let s: String = speaker.chars().filter(|&c| c != ' ').collect();
    ["교역소", "역소주", "소주인", "역소의도", "소의도제"].iter().any(|k| s.contains(k))
}

/// 대사가 끝난 모양인가: `[.!?]$|(다|요|어|야|지|네)[.!]?$`
pub fn ends_sentence(t: &str) -> bool {
    let c: Vec<char> = t.chars().collect();
    let Some(&last) = c.last() else { return false };
    if ".!?".contains(last) {
        return true;
    }
    let end = "다요어야지네";
    end.contains(last) || (".!".contains(last) && c.len() >= 2 && end.contains(c[c.len() - 2]))
}

/// Python round(): 반올림 0.5 는 짝수 쪽
pub fn round_half_even(v: f64) -> i64 {
    let r = v.round();
    if (v - v.trunc()).abs() == 0.5 { (2.0 * (v / 2.0).round()) as i64 } else { r as i64 }
}
