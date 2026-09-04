//! Снимает тот же вывод, что даёт Go-версия, для побайтовой сверки перевода.
use gf_data_1c::refusal::{Kind, Refusal};

fn main() {
    let e = Refusal::new(Kind::BaseError, "запрос не выполнен", "поле не найдено")
        .hint("подсказка раз")
        .hint("подсказка два")
        .stamp("ut11");
    println!("=== 1 ===");
    println!("{e}");
    println!("=== 2 ===");
    let known = vec!["ut11".to_string(), "bu3".to_string()];
    println!("{}", Refusal::unknown_base("нетбазы", &known));
    println!("=== 3 ===");
    println!("{}", Refusal::unknown_base("любая", &[]));
}
