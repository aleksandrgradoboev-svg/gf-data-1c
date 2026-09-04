//! Читает НАСТОЯЩИЙ реестр, записанный Go-версией: перевод обязан не осиротить его.
//! Только чтение — save() здесь не зовётся.
fn main() {
    let path = std::env::args().nth(1).expect("путь к реестру аргументом");
    let path = std::path::PathBuf::from(path);
    match gf_data_1c::registry::Registry::load(Some(&path)) {
        Ok(r) => {
            println!("баз прочитано: {}", r.bases.len());
            for b in &r.bases {
                let пароль = match b.reveal_password() {
                    Ok(p) if p.is_empty() => "(пусто)".to_string(),
                    Ok(p) => format!("расшифрован, {} знаков", p.chars().count()),
                    Err(e) => format!("НЕ РАСШИФРОВАН: {e}"),
                };
                println!(
                    " - {} | url={} | user={} | пароль: {}",
                    b.name, b.url, b.user, пароль
                );
            }
            println!("имена по порядку: {:?}", r.names());
            match r.resolve("") {
                Ok(_) => println!("ОШИБКА: пустое имя не должно разрешаться"),
                Err(e) => println!(
                    "пустое имя → отказ (верно): {}",
                    e.to_string().lines().next().unwrap()
                ),
            }
        }
        Err(e) => println!("ОТКАЗ: {e}"),
    }
}
