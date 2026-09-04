//! Вывод bases на ЖИВОМ реестре — сверка с Go. Этот текст читает модель.
fn main() {
    let s = gf_data_1c::tools::Set::new(gf_data_1c::server::VERSION); // путь пуст → умолчание, живой файл
    match s.bases(&gf_data_1c::tools::bases::BasesInput {
        action: "list".into(),
        ..Default::default()
    }) {
        Ok(t) => println!("{t}"),
        Err(e) => println!("ОТКАЗ: {e}"),
    }
}
