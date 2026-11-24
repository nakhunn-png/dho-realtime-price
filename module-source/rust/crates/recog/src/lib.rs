//! 헬폴 감지기 화면 인식 (collector/recog.py 의 Rust 판)
//!
//! - 교역 창 위치: 대화상자 왼쪽 위 모서리 틀 맞추기 (해상도·창 위치와 무관)
//! - 품목명 / 도시명: 후보 이름을 굴림 14px로 그려서 화면 글자와 정규화 상관 비교
//! - 가격·%: 게임 숫자 글자 틀과 글자 단위 비교
//! - 교역소 주인 대사: 정해진 대사를 굴림 14px로 그려 낱말 단위 비교 (못 맞추면 글자 인식)
//! - 구입/매각 구분, 목록에 없는 지명: Windows 내장 한국어 글자 인식
pub mod arrows;
pub mod cv;
pub mod digits;
pub mod font;
pub mod icons;
pub mod names;
pub mod nanban;
pub mod nav;
pub mod ocr;
pub mod owner_font;
pub mod pngio;
pub mod recognizer;
pub mod text;

pub use cv::Rgb;
pub use recognizer::{NearRow, Pct, Recognizer, TradeRow, TradeScreen};

mod assets {
    include!(concat!(env!("OUT_DIR"), "/assets.rs"));
}
