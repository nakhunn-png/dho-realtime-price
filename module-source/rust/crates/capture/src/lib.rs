//! 대항해시대 온라인 창을 Windows Graphics Capture로 복사한다 (collector/capture.py 의 Rust 판).
//!
//! - 다른 창에 가려져 있어도 복사된다 (최소화된 창만 안 됨)
//! - 게임 프로세스에는 아무것도 하지 않는다: 화면에 그려진 결과만 받는다
pub mod win;
mod wgc;

pub use wgc::{Frame, WindowCapture};
pub use win::*;

use std::sync::OnceLock;

static LOG: OnceLock<Box<dyn Fn(&str) + Send + Sync>> = OnceLock::new();

/// 로그 줄을 받을 곳 (정하지 않으면 표준 출력)
pub fn set_log(f: impl Fn(&str) + Send + Sync + 'static) {
    let _ = LOG.set(Box::new(f));
}

pub(crate) fn log(msg: &str) {
    match LOG.get() {
        Some(f) => f(msg),
        None => println!("{msg}"),
    }
}

/// 같은 진단 문장은 로그에 한 번만 (원인을 알려 주려는 줄이라 반복하지 않는다)
pub fn say_once(msg: &str) {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static SAID: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let mut g = SAID.lock().unwrap();
    if g.get_or_insert_with(HashSet::new).insert(msg.to_string()) {
        drop(g);
        log(msg);
    }
}
