//! Читает пароль, зашифрованный Go-версией: реестр обязан пережить смену реализации.
use std::io::Read;

fn main() {
    let mut закрытый = String::new();
    std::io::stdin().read_to_string(&mut закрытый).unwrap();
    let закрытый = закрытый.trim();
    match gf_data_1c::secret::reveal(закрытый) {
        Ok(v) => println!("{v}"),
        Err(e) => println!("ОТКАЗ: {e}"),
    }
}
