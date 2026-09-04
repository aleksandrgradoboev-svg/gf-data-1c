//! Данные на живой базе — сверка с Go.
use gf_data_1c::refusal::Refusal;
use gf_data_1c::tools::data::*;
use gf_data_1c::tools::Set;

fn вывод(r: Result<String, Refusal>) {
    match r {
        Ok(t) => println!("{t}"),
        Err(e) => println!("{e}"),
    }
}

fn main() {
    // allow_raw_query: сверяем печать и отказы, а не гейт — он проверен отдельно.
    let mut s = Set::new("0.1.0");
    s.allow_raw_query = true;

    println!("=== count ===");
    вывод(s.count(&CountInput {
        base: "bu3".into(),
        table: "Справочник.Валюты".into(),
        ..Default::default()
    }));

    println!("=== count с отбором ===");
    вывод(s.count(&CountInput {
        base: "bu3".into(),
        table: "Справочник.Валюты".into(),
        where_: "НЕ ПометкаУдаления".into(),
        ..Default::default()
    }));

    println!("=== query_check ===");
    вывод(s.query_check(&QueryCheckInput {
        base: "bu3".into(),
        query: "ВЫБРАТЬ Код, Наименование ИЗ Справочник.Валюты".into(),
    }));

    println!("=== query ===");
    вывод(s.query(&QueryInput {
        base: "bu3".into(),
        query: "ВЫБРАТЬ Код, Наименование ИЗ Справочник.Валюты УПОРЯДОЧИТЬ ПО Код".into(),
        limit: 3,
        ..Default::default()
    }));

    println!("=== query: отказ платформы ===");
    вывод(s.query(&QueryInput {
        base: "bu3".into(),
        query: "ВЫБРАТЬ Чепуха ИЗ Справочник.Валюты".into(),
        ..Default::default()
    }));

    println!("=== query: пустой результат ===");
    вывод(s.query(&QueryInput {
        base: "bu3".into(),
        query: r#"ВЫБРАТЬ Код ИЗ Справочник.Валюты ГДЕ Код = "нетакого""#.into(),
        ..Default::default()
    }));
}
