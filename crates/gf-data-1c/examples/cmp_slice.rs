//! Срез и бухгалтерские итоги на живой базе — сверка с Go.
use gf_data_1c::refusal::Refusal;
use gf_data_1c::tools::slice::*;
use gf_data_1c::tools::Set;

fn вывод(r: Result<String, Refusal>) {
    match r {
        Ok(t) => println!("{t}"),
        Err(e) => println!("{e}"),
    }
}

fn main() {
    let s = Set::new("0.1.0");

    println!("=== accounts: остатки по 41 ===");
    вывод(s.accounts(&AccountsInput {
        base: "bu3".into(),
        account: "41".into(),
        ..Default::default()
    }));

    println!("=== accounts: несуществующий счёт ===");
    вывод(s.accounts(&AccountsInput {
        base: "bu3".into(),
        account: "9999".into(),
        ..Default::default()
    }));

    println!("=== slice: непериодический или отсутствующий регистр ===");
    вывод(s.slice(&SliceInput {
        base: "bu3".into(),
        register: "НетТакогоРегистра".into(),
        ..Default::default()
    }));
}
