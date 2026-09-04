//! Живая проверка КЛАССИФИКАЦИИ ОТКАЗОВ: четыре беды должны различаться.
use gf_data_1c::channel::Client;
use gf_data_1c::registry::{Base, Registry};

fn проба(имя: &str, base: Base, метод: &str) {
    println!("── {имя} ──");
    match Client::new(base, Some(std::time::Duration::from_secs(10)))
        .unwrap()
        .get(метод, &[])
    {
        Ok(d) => println!(
            "ответ: {}",
            String::from_utf8_lossy(&d)
                .chars()
                .take(80)
                .collect::<String>()
        ),
        Err(e) => println!("{:?}: {}", e.kind, e.to_string().lines().next().unwrap()),
    }
    println!();
}

fn main() {
    let reg = Registry::load(None).unwrap();
    let живая = reg.resolve("bu3").expect("база bu3 нужна для пробы");

    // 1. Погашенный веб-сервер: заведомо свободный порт.
    let мёртвый = Base {
        name: "проба".into(),
        url: "http://127.0.0.1:9/hs/gt-data".into(),
        ..Default::default()
    };
    проба("веб-сервер не поднят", мёртвый, "version");

    // 2. Нет публикации: тот же сервер, чужое имя базы → HTML-страница веб-сервера.
    let нет_публикации = Base {
        name: "проба".into(),
        url: "http://localhost:8081/нет-такой/hs/gt-data".into(),
        ..живая.clone()
    };
    проба("базы нет по адресу", нет_публикации, "version");

    // 3. Нет маршрута в самой 1С: живая база, выдуманный метод.
    проба("маршрута нет в 1С", живая.clone(), "нет-такого-метода");

    // 4. Отказ прав: живой адрес, неверный пароль.
    let плохой_пароль = Base {
        password: "заведомо-неверный".into(),
        ..живая.clone()
    };
    проба("отказ прав", плохой_пароль, "version");
}
