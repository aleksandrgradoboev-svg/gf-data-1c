//! Живая проба канала против настоящей базы: только чтение, метод version.
fn main() {
    let reg = gf_data_1c::registry::Registry::load(None).expect("реестр не прочитан");
    let base = match reg.resolve("bu3") {
        Ok(b) => b,
        Err(e) => {
            println!("{e}");
            return;
        }
    };
    println!("база: {} → {}", base.name, base.url);

    let c = gf_data_1c::channel::Client::new(base, None).expect("канал не создан");
    match c.get("version", &[]) {
        Ok(data) => println!("ОТВЕТ: {}", String::from_utf8_lossy(&data)),
        Err(e) => println!("{e}"),
    }
}
