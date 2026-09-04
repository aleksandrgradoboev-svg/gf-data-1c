//! Формат файла реестра: Go-версия должна прочитать то, что записал Rust.
use gf_data_1c::registry::{Base, Registry};

fn main() {
    let p = std::env::temp_dir().join("cmp-reg-rust.json");
    let _ = std::fs::remove_file(&p);
    let mut r = Registry::load(Some(&p)).unwrap();
    r.add(Base {
        name: "ut11".into(),
        title: "Торговля".into(),
        url: "http://localhost:8081/ut11/hs/gt-data".into(),
        user: "agent".into(),
        password: "пароль".into(),
        auth: "ntlm".into(),
    })
    .unwrap();
    r.add(Base {
        name: "bu3".into(),
        url: "http://localhost:8081/bu3/hs/gt-data".into(),
        ..Default::default()
    })
    .unwrap();
    print!("{}", std::fs::read_to_string(&p).unwrap());
}
