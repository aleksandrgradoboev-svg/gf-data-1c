//! Форма строки журнала и путь по умолчанию — сверка с Go-версией.
fn main() {
    let p = std::env::temp_dir().join("cmp-journal-rust.log");
    let _ = std::fs::remove_file(&p);
    gf_data_1c::journal::open(Some(&p)).expect("журнал не открылся");
    gf_data_1c::journal::writef(format_args!(
        "ut11 GET http://localhost:8081/ut11/hs/gt-data/version → {} за {}",
        200, "31ms"
    ));
    gf_data_1c::journal::close();
    let text = std::fs::read_to_string(&p).unwrap();
    println!("{text:?}");
    println!(
        "путь по умолчанию: {}",
        gf_data_1c::journal::default_path().display()
    );
}
