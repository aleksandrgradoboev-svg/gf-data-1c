//! Живая выгрузка в файл: и csv, и jsonl.
use gf_data_1c::tools::export::ExportInput;
use gf_data_1c::tools::Set;

fn main() {
    let mut s = Set::new("0.1.0");
    s.allow_raw_query = true;
    let dir = std::env::temp_dir().join("gfdata-export-live");
    let _ = std::fs::create_dir_all(&dir);

    for формат in ["csv", "jsonl"] {
        let path = dir.join(format!("rust.{формат}"));
        let _ = std::fs::remove_file(&path);
        let r = s.export(&ExportInput {
            base: "bu3".into(),
            query: "ВЫБРАТЬ Код, Наименование ИЗ Справочник.Валюты УПОРЯДОЧИТЬ ПО Код".into(),
            format: формат.into(),
            path: path.to_string_lossy().to_string(),
            ..Default::default()
        });
        match r {
            Ok(t) => {
                // Печатаем только первую строку отчёта: путь и время у реализаций разные.
                println!("=== {формат} ===");
                println!("{}", t.lines().next().unwrap());
                println!("--- содержимое ---");
                print!("{}", std::fs::read_to_string(&path).unwrap());
            }
            Err(e) => println!("{e}"),
        }
    }
}
