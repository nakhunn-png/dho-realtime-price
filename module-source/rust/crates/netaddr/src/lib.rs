//! 게임이 접속해 있는 서버 주소로 '지금 들어가 있는 서버'를 안다 (collector/netaddr.py 의 Rust 판).
//!
//! 같은 서버(헬레네·폴라리스)에 들어가면 누구든 같은 주소에 접속한다. 다른 서버를 방문하면 방문한 서버 주소로 바뀐다.
//! 주소표: web/config.js 의 addrs·ranges (처음부터 아는 주소) + 공유 서버 /addr (PC들이 알아낸 주소) + data/addrs.json (이 PC가 알아낸 주소)
//! 게임 프로세스에는 아무것도 하지 않는다: Windows 의 연결 목록만 읽는다 (관리자 권한 필요 없음).
mod book;
mod sys;

pub use book::{AddrBook, addr_key, ip_of, key_addr, parse_ranges};
pub use sys::{all_rows, game_pids, remote_addrs, thread_window_pid, window_pid, window_pid_direct};

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
