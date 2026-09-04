//! Команда `gfdata` — MCP-сервер доступа к данным информационных баз 1С:Предприятие.
//!
//! Запускается агентом как дочерний процесс и общается по stdio. Регистрация у агента
//! сводится к пути этого бинарника: реестр баз лежит в профиле пользователя, флаги
//! нужны только для нештатных случаев.

use std::io::{BufRead, Write};

use gf_data_1c::{installer, journal, registry, server};

/// Разобранные аргументы командной строки.
///
/// Свой разбор, а не крейт: флагов полтора десятка, все простые, и зависимость ради
/// них стоила бы дороже — тем более в продукте, который целится в реестр отечественного
/// ПО, где каждая внешняя библиотека это отдельная строка в описании состава.
#[derive(Default)]
struct Args {
    registry: String,
    timeout: String,
    log: String,
    version: bool,
    help: bool,
    http: String,
    token: String,
    install: String,
    server_base: bool,
    db_user: String,
    db_password: String,
    platform: String,
    export_extension: String,
    /// Нераспознанный флаг — называется в отказе, а не проглатывается.
    unknown: Vec<String>,
}

const USAGE: &str = "gfdata — сервер данных 1С по протоколу MCP.

Без флагов: stdio-режим, для запуска агентом как дочерний процесс.

  -registry <путь>     реестр баз (умолчание — профиль пользователя)
  -timeout <секунды>   таймаут обращения к базе
  -log <путь|auto>     вести журнал сервера в файл
  -version             напечатать версию и выйти
  -help                эта справка

Сетевой режим:
  -http <адрес|auto>   слушать адрес вместо stdio (умолчание auto — 127.0.0.1:9077)
  -token <строка>      требовать заголовок Authorization: Bearer <токен>

Установка расширения в базу:
  -install <база>      путь к файловой базе или строка сервер\\база вместе с -server
  -server              значение -install — строка подключения к серверной базе
  -db-user <имя>       пользователь базы для конфигуратора
  -db-password <пароль>
  -platform <путь>     путь к 1cv8.exe (умолчание — старшая найденная версия)
  -export-extension <путь>  выложить расширение файлом .cfe и выйти";

fn parse_args(argv: &[String]) -> Args {
    let mut a = Args::default();
    let mut i = 0;
    while i < argv.len() {
        let arg = argv[i].as_str();
        // Принимаются обе формы, -flag и --flag: агенты пишут по-разному, а отказ
        // из-за лишней чёрточки выглядит как поломка сервера.
        let name = arg.trim_start_matches('-');
        // Форма -flag=value наравне с -flag value.
        let (name, inline) = match name.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (name, None),
        };
        let mut value = || -> String {
            if let Some(v) = inline.clone() {
                return v;
            }
            i += 1;
            argv.get(i).cloned().unwrap_or_default()
        };
        match name {
            "registry" => a.registry = value(),
            "timeout" => a.timeout = value(),
            "log" => a.log = value(),
            "version" | "v" => a.version = true,
            "help" | "h" => a.help = true,
            "http" => a.http = value(),
            "token" => a.token = value(),
            "install" => a.install = value(),
            "server" => a.server_base = true,
            "db-user" => a.db_user = value(),
            "db-password" => a.db_password = value(),
            "platform" => a.platform = value(),
            "export-extension" => a.export_extension = value(),
            _ => a.unknown.push(arg.to_string()),
        }
        i += 1;
    }
    a
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = parse_args(&argv);

    if !args.unknown.is_empty() {
        eprintln!(
            "gf-data-1c: неизвестные флаги: {}\n\n{USAGE}",
            args.unknown.join(", ")
        );
        std::process::exit(2);
    }
    if args.help {
        println!("{USAGE}");
        return;
    }

    if !args.export_extension.is_empty() {
        // Отдельный режим, а не часть установки: расширение подключает администратор
        // базы, а работает под каналом обычный пользователь — установка через
        // конфигуратор ему недоступна.
        match installer::export(&args.export_extension, &args.platform) {
            Ok(path) => {
                println!(
                    "Расширение {} выгружено: {}",
                    installer::EXTENSION_NAME,
                    path.display()
                );
                println!(
                    "Подключить: Конфигуратор → Конфигурация → Расширения конфигурации → \
                     Добавить из файла."
                );
            }
            Err(e) => {
                eprintln!("gf-data-1c: {e}");
                // Платформы нет — исходники всё же выложены, и это сказано вслух;
                // такой исход не провал, а другой формат.
                let код = i32::from(!e.what.contains("платформа не найдена"));
                std::process::exit(код);
            }
        }
        return;
    }

    if !args.install.is_empty() {
        let opts = installer::Options {
            base: args.install.clone(),
            server: args.server_base,
            user: args.db_user.clone(),
            password: args.db_password.clone(),
            platform: args.platform.clone(),
        };
        if let Err(e) = installer::install(&opts) {
            eprintln!("gf-data-1c: расширение не установлено: {e}");
            std::process::exit(1);
        }
        println!(
            "Расширение {} установлено в базу {}.",
            installer::EXTENSION_NAME,
            args.install
        );
        println!(
            "Дальше: опубликуйте базу на веб-сервере и зарегистрируйте её \
             инструментом bases (action=add)."
        );
        return;
    }

    if args.version {
        println!("gf-data-1c {}", server::VERSION);
        println!("реестр баз: {}", registry_default(&args.registry));
        println!("журнал по умолчанию: {}", journal::default_path().display());
        return;
    }

    if !args.log.is_empty() {
        let path = if args.log == "auto" {
            journal::default_path()
        } else {
            std::path::PathBuf::from(&args.log)
        };
        // Журнал — удобство, а не условие работы: сказать и продолжить.
        if let Err(e) = journal::open(Some(&path)) {
            eprintln!("gf-data-1c: журнал не открыт ({e}), работаю без него");
        }
    }

    let options = server::Options {
        registry_path: if args.registry.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(&args.registry))
        },
        timeout: parse_timeout(&args.timeout),
        allow_raw_query: false,
    };

    // Чтение реестра при старте нужно ради побочного действия: пароли, оставшиеся
    // открытыми (вписанные руками или от прежней версии), защищаются сразу, а не при
    // ближайшем изменении реестра — которого может не случиться месяцами.
    if let Err(e) = registry::Registry::load(options.registry_path.as_deref()) {
        eprintln!("gf-data-1c: реестр баз не прочитан: {e}");
    }

    if !args.http.is_empty() {
        let addr = if args.http == "auto" {
            gf_data_1c::http::DEFAULT_ADDR.to_string()
        } else {
            args.http.clone()
        };
        if let Err(e) = gf_data_1c::http::serve(options, &addr, &args.token) {
            eprintln!("gf-data-1c: сетевой режим остановлен: {e}");
            std::process::exit(1);
        }
        return;
    }

    if let Err(e) = serve_stdio(options) {
        eprintln!("gf-data-1c: сервер остановлен: {e}");
        std::process::exit(1);
    }
}

/// Цикл stdio: строка на входе — строка на выходе.
///
/// Печать в стандартный вывод здесь и есть протокол, поэтому всё служебное идёт
/// в stderr. Одна лишняя строка в stdout ломает связь с агентом, и ломает молча.
fn serve_stdio(options: server::Options) -> std::io::Result<()> {
    let mut srv = server::Server::new(options);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    journal::writef(format_args!("stdio-режим: сервер запущен"));

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(ответ) = srv.handle(&line) {
            writeln!(stdout, "{ответ}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// Таймаут в секундах. Пусто или мусор — умолчание канала.
fn parse_timeout(s: &str) -> Option<std::time::Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Принимается и «30», и «30s»: Go-версия печатала длительность с суффиксом,
    // и такой же строкой её могли вписать в конфигурацию агента.
    let число = s.trim_end_matches(['s', 'с']);
    число
        .parse::<u64>()
        .ok()
        .map(std::time::Duration::from_secs)
}

fn registry_default(path: &str) -> String {
    if !path.is_empty() {
        return path.to_string();
    }
    registry::default_path().display().to_string()
}
