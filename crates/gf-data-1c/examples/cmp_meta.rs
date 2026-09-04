//! Три инструмента метаданных на живой базе — сверка с Go.
use gf_data_1c::tools::meta::*;
use gf_data_1c::tools::Set;

fn печать(r: Result<String, gf_data_1c::refusal::Refusal>) {
    match r {
        Ok(t) => println!("{t}"),
        Err(e) => println!("{e}"),
    }
}

fn main() {
    let s = Set::new("0.1.0");

    println!("=== base_info ===");
    печать(s.base_info(&BaseInfoInput { base: "bu3".into() }));

    println!("=== metadata (сводка) ===");
    печать(s.metadata(&MetadataInput {
        base: "bu3".into(),
        ..Default::default()
    }));

    println!("=== object: регистр бухгалтерии ===");
    печать(s.object(&ObjectInput {
        base: "bu3".into(),
        object_type: "AccountingRegister".into(),
        object_name: "Хозрасчетный".into(),
    }));

    println!("=== object: не найден ===");
    печать(s.object(&ObjectInput {
        base: "bu3".into(),
        object_type: "Document".into(),
        object_name: "ЧепухаКоторойНет".into(),
    }));
}
