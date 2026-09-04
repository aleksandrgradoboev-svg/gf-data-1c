//! Проба на ЖИВОМ реестре — сверка с Go.
fn main() {
    let s = gf_data_1c::tools::Set::new(gf_data_1c::server::VERSION);
    match s.probe(&gf_data_1c::tools::probe::ProbeInput::default()) {
        Ok(t) => println!("{t}"),
        Err(e) => println!("ОТКАЗ: {e}"),
    }
}
