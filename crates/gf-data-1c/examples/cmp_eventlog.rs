//! Журнал регистрации на живой базе — сверка с Go.
use gf_data_1c::tools::data::EventLogInput;
use gf_data_1c::tools::Set;

fn вывод(s: &Set, in_: EventLogInput) {
    match s.event_log(&in_) {
        Ok(t) => println!("{t}"),
        Err(e) => println!("{e}"),
    }
}

fn main() {
    let s = Set::new("0.1.0");
    println!("=== последние 5 ===");
    вывод(
        &s,
        EventLogInput {
            base: "bu3".into(),
            limit: 5,
            ..Default::default()
        },
    );
    println!("=== только ошибки, 3 ===");
    вывод(
        &s,
        EventLogInput {
            base: "bu3".into(),
            level: "Ошибка".into(),
            limit: 3,
            ..Default::default()
        },
    );
    println!("=== неизвестная база ===");
    вывод(
        &s,
        EventLogInput {
            base: "нетбазы".into(),
            ..Default::default()
        },
    );
}
