//! Проба на ЖИВОМ реестре — сверка с Go.
fn main() {
    let s = gf_data_1c::tools::Set::new("0.1.0");
    match s.probe(&gf_data_1c::tools::probe::ProbeInput::default()) {
        Ok(t) => println!("{t}"),
        Err(e) => println!("ОТКАЗ: {e}"),
    }
}
