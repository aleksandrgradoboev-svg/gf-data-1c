//! Журнал сервера в файл.
//!
//! Агент видит только ответы инструментов, а причина отказа часто лежит уровнем ниже:
//! какой адрес запрашивали, что ответил веб-сервер, сколько это заняло. Без файла эта
//! половина картины пропадает, и разбор жалобы «оно не работает» начинается с нуля.
//!
//! Секреты сюда не попадают: пишется адрес без учётных данных, они и в реестре хранятся
//! отдельно от URL.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Журнал общий на процесс — как и было в версии на Go. Это осознанно: писать в один
/// файл из разных мест без общего замка нельзя, а заводить по журналу на сессию значит
/// потерять причину отказа, случившегося до её открытия.
fn state() -> &'static Mutex<Option<File>> {
    static STATE: OnceLock<Mutex<Option<File>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

/// Журнал рядом с реестром баз, в профиле пользователя.
///
/// Каталог `gt-data-1c` прежний — см. `registry::default_path`: переименование продукта
/// не двигает данные уже работающих установок.
pub fn default_path() -> PathBuf {
    let dir = dirs_cache().unwrap_or_else(|| PathBuf::from("."));
    dir.join("gt-data-1c").join("server.log")
}

/// Каталог кэша пользователя: `%LocalAppData%` в Windows, `$XDG_CACHE_HOME` или
/// `~/.cache` в прочих системах — то же, что даёт `os.UserCacheDir` в Go.
fn dirs_cache() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
    }
}

/// Включает журнал.
///
/// Ошибка открытия возвращается вызывающему, но валить сервер ею нельзя: работать без
/// журнала можно, а падать из-за него на ровном месте — нет.
pub fn open(path: Option<&Path>) -> std::io::Result<()> {
    let path = match path {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => default_path(),
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = OpenOptions::new().append(true).create(true).open(&path)?;

    let mut guard = state().lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(file);
    Ok(())
}

/// Закрывает журнал.
pub fn close() {
    let mut guard = state().lock().unwrap_or_else(|e| e.into_inner());
    *guard = None;
}

/// Ведётся ли журнал.
pub fn enabled() -> bool {
    state().lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// Пишет строку журнала с отметкой времени.
///
/// Ошибка записи глотается намеренно: журнал — вспомогательная вещь, и падать из-за
/// переполненного диска, отвечая на вопрос о базе, продукт не должен.
pub fn write(line: &str) {
    let mut guard = state().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(file) = guard.as_mut() {
        let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(file, "{stamp}  {line}");
    }
}

/// То же, что [`write`], но с форматированием: `journal::writef(format_args!(...))`.
/// Отдельная функция, чтобы не собирать строку, когда журнал выключен.
pub fn writef(args: std::fmt::Arguments<'_>) {
    if enabled() {
        write(&args.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn путь_по_умолчанию_ведёт_в_прежний_каталог() {
        let p = default_path();
        let s = p.to_string_lossy();
        assert!(
            s.contains("gt-data-1c"),
            "каталог данных не двигается при переименовании продукта: {s}"
        );
        assert!(s.ends_with("server.log"), "{s}");
    }

    #[test]
    fn выключенный_журнал_молчит_и_не_падает() {
        close();
        assert!(!enabled());
        write("эта строка никуда не идёт");
    }

    #[test]
    fn запись_идёт_с_отметкой_времени() {
        let dir = std::env::temp_dir().join(format!("gfdata-journal-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("server.log");
        let _ = std::fs::remove_file(&path);

        open(Some(&path)).expect("журнал не открылся");
        write("проба пера");
        close();

        let text = std::fs::read_to_string(&path).expect("журнал не прочитан");
        assert!(text.contains("проба пера"), "{text}");
        // Отметка времени: «2026-09-04 10:41:05  проба пера» — 19 знаков даты и времени.
        let первая = text.lines().next().unwrap();
        assert!(
            первая.len() > 19 && первая.as_bytes()[4] == b'-' && первая.as_bytes()[13] == b':',
            "формат отметки времени разошёлся с прежним: {первая}"
        );
        let _ = std::fs::remove_file(&path);
    }
}
