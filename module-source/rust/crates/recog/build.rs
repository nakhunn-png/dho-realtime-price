// assets/ 의 그림 틀을 실행 파일 안에 넣는 목록을 만든다 (digits/*.png, arrows/*.png)
use std::{env, fs, path::Path};

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let mut out = String::new();
    for sub in ["digits", "arrows", "nanban"] {
        let mut files: Vec<_> = fs::read_dir(dir.join(sub)).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "png")).collect();
        files.sort();
        out += &format!("pub static {}: &[(&str, &[u8])] = &[\n", sub.to_uppercase());
        for p in files {
            let name = p.file_name().unwrap().to_str().unwrap();
            out += &format!("    ({name:?}, include_bytes!({:?})),\n", p.to_str().unwrap());
        }
        out += "];\n";
    }
    out += &format!("pub static DIALOG_CORNER: &[u8] = include_bytes!({:?});\n", dir.join("dialog_corner.png").to_str().unwrap());
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("assets.rs"), out).unwrap();
    println!("cargo:rerun-if-changed=assets");
}
