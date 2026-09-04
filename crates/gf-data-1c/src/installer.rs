//! Установка расширения доступа к данным в информационную базу 1С.
//!
//! Исходники расширения лежат внутри бинаря: это возможно ровно потому, что расширение
//! не заимствует язык расширяемой конфигурации и собрано с низким режимом совместимости —
//! одна и та же сборка встаёт в любую базу. Привязанное к конфигурации расширение
//! пришлось бы собирать на месте, а для этого нужна выгрузка целевой конфигурации.

use std::path::{Path, PathBuf};
use std::process::Command;

use include_dir::{include_dir, Dir};

use crate::refusal::{Kind, Refusal};

/// Встроенные исходники расширения — аналог `go:embed extension`.
///
/// Каталог заполняется отдельным шагом сборки и в репозиторий не едет: это артефакт.
/// Как и `go:embed`, макрос требует, чтобы каталог существовал во время компиляции,
/// поэтому в нём лежит файл-заглушка — без него свежий клон не собирался бы вовсе.
static EXTENSION: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../internal/installer/extension");

/// Имя расширения в базе. По нему же оно обновляется и удаляется.
pub const EXTENSION_NAME: &str = "GTData";

/// Что и куда ставим.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Файловая база (путь к каталогу) либо, при `server`, строка «сервер\база».
    pub base: String,
    pub server: bool,
    /// Учётные данные ПОЛЬЗОВАТЕЛЯ БАЗЫ для конфигуратора, а не веб-сервиса:
    /// конфигуратор ходит в базу напрямую.
    pub user: String,
    pub password: String,
    /// Путь к `1cv8.exe`; пусто — ищем сами.
    pub platform: String,
}

/// Текст отказа, когда расширение не встроено в бинарь.
///
/// Каталог `extension/` заполняется отдельным шагом сборки и в репозиторий не едет: это
/// артефакт. Из-за этого свежий клон собирается, но установщику работать нечем, и без
/// этой проверки беда вскрывалась бы отказом конфигуратора про «принадлежность основного
/// объекта конфигурации» — сообщением, которое отправляет искать причину не туда.
pub const EXTENSION_NOT_BUILT: &str =
    "расширение не встроено в этот бинарь: каталог internal/installer/extension пуст.\n\
     Соберите расширение и пересоберите сервер:\n\
     \x20 powershell -File build/build-extension.ps1 -OutputDir internal/installer/extension\n\
     \x20 cargo build --release\n\
     Бинарь со страницы релизов расширение уже несёт";

/// Встроено ли расширение.
///
/// Признак — `Configuration.xml`: главный файл выгрузки, без него остальное бессмысленно.
pub fn extension_built() -> bool {
    EXTENSION.get_file("Configuration.xml").is_some()
}

/// Разворачивает встроенные исходники во временный каталог.
///
/// Возвращает путь; удалять его — забота вызывающего (в Go это `defer os.RemoveAll`).
fn unpack() -> Result<PathBuf, Refusal> {
    if !extension_built() {
        return Err(Refusal::new(
            Kind::BadRequest,
            "расширение не встроено",
            EXTENSION_NOT_BUILT,
        ));
    }
    let dir = std::env::temp_dir().join(format!(
        "gf-data-1c-ext-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir)
        .map_err(|e| Refusal::new(Kind::Internal, "временный каталог не создан", e.to_string()))?;
    if let Err(e) = copy_dir(&EXTENSION, &dir) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(dir)
}

/// Раскладывает встроенный каталог на диск.
fn copy_dir(src: &Dir<'_>, dst: &Path) -> Result<(), Refusal> {
    for file in src.files() {
        let name = file
            .path()
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let to = dst.join(&name);
        std::fs::write(&to, file.contents()).map_err(|e| {
            Refusal::new(
                Kind::Internal,
                format!("файл {name} не записан"),
                e.to_string(),
            )
        })?;
    }
    for sub in src.dirs() {
        let name = sub
            .path()
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let to = dst.join(&name);
        std::fs::create_dir_all(&to).map_err(|e| {
            Refusal::new(
                Kind::Internal,
                format!("каталог {name} не создан"),
                e.to_string(),
            )
        })?;
        copy_dir(sub, &to)?;
    }
    Ok(())
}

/// Разворачивает расширение и загружает его конфигуратором.
pub fn install(opts: &Options) -> Result<(), Refusal> {
    if opts.base.trim().is_empty() {
        return Err(Refusal::new(
            Kind::BadRequest,
            "база не названа",
            "укажите путь к файловой базе или строку сервер\\база с флагом -server",
        ));
    }

    let platform = resolve_platform(&opts.platform)?;
    let dir = unpack()?;
    // Каталог удаляется в любом исходе: держатель снимает его на выходе из функции.
    let _guard = TempDir(dir.clone());

    let log_file = dir.join("designer.log");
    let mut args: Vec<String> = vec!["DESIGNER".into()];
    if opts.server {
        args.push("/S".into());
        args.push(opts.base.clone());
    } else {
        args.push("/F".into());
        args.push(opts.base.clone());
    }
    if !opts.user.is_empty() {
        args.push(format!("/N{}", opts.user));
    }
    if !opts.password.is_empty() {
        args.push(format!("/P{}", opts.password));
    }
    args.extend([
        "/LoadConfigFromFiles".to_string(),
        dir.to_string_lossy().to_string(),
        "-Format".into(),
        "Hierarchical".into(),
        "-Extension".into(),
        EXTENSION_NAME.into(),
        "/UpdateDBCfg".into(),
        "/Out".into(),
        log_file.to_string_lossy().to_string(),
        "/DisableStartupDialogs".into(),
    ]);

    let run = Command::new(&platform).args(&args).status();

    // Код возврата конфигуратора — слабое доказательство: 1cv8.exe оконное приложение
    // и умеет отрапортовать успехом, застряв на диалоге. Поэтому решает журнал.
    //
    // Пустой журнал означает успех: конфигуратор пишет в /Out только то, что пошло не так.
    // Файл при этом не нулевой — в нём стоит метка кодировки, и её надо снять,
    // иначе успешная установка читается как ошибка.
    let report = std::fs::read(&log_file).unwrap_or_default();
    let report = report
        .strip_prefix(&[0xEF, 0xBB, 0xBF][..])
        .unwrap_or(&report);
    let text = String::from_utf8_lossy(report).trim().to_string();

    let failed = match run {
        Err(e) => Some(e.to_string()),
        Ok(st) if !st.success() => Some(format!("код возврата {st}")),
        Ok(_) => None,
    };
    if let Some(why) = failed {
        return Err(if text.is_empty() {
            Refusal::new(Kind::Internal, "конфигуратор отказал", why)
        } else {
            Refusal::new(Kind::Internal, "конфигуратор отказал", why).hint(text)
        });
    }
    if !text.is_empty() {
        return Err(Refusal::new(
            Kind::Internal,
            "конфигуратор завершился без ошибки, но журнал не пуст",
            text,
        ));
    }
    Ok(())
}

/// Выкладывает встроенное расширение на диск, чтобы его подключил администратор базы
/// своими руками.
///
/// Нужно потому, что установка через `install` требует режима конфигуратора, то есть права
/// «Администрирование» у пользователя базы. Там, где расширение ставит админ, а работает
/// под каналом обычный пользователь, файл нужен отдельно — а он вшит в бинарь и достать его
/// неоткуда.
///
/// Отдаётся `.cfe`, когда платформа найдена: именно его принимает форма «Расширения
/// конфигурации». Платформы нет — выкладываются XML-исходники, и об этом говорится прямо,
/// а не подсовывается молча не тот формат.
pub fn export(dst: &str, platform: &str) -> Result<PathBuf, Refusal> {
    if dst.trim().is_empty() {
        return Err(Refusal::new(
            Kind::BadRequest,
            "не сказано, куда выгружать расширение",
            "укажите путь к файлу .cfe",
        ));
    }

    let src = unpack()?;
    let _guard = TempDir(src.clone());

    let exe = match resolve_platform(platform) {
        Ok(e) => e,
        Err(plat_err) => {
            // Без платформы .cfe не собрать: это делает конфигуратор. Отдаём исходники —
            // они грузятся тем же конфигуратором на машине, где платформа есть.
            let out = PathBuf::from(dst.strip_suffix(".cfe").unwrap_or(dst));
            std::fs::create_dir_all(&out).map_err(|e| {
                Refusal::new(
                    Kind::Internal,
                    format!("каталог {} не создан", out.display()),
                    e.to_string(),
                )
            })?;
            copy_dir(&EXTENSION, &out)?;
            return Err(Refusal::new(
                Kind::BadRequest,
                format!(
                    "платформа не найдена, собран не .cfe, а XML-исходники в {}",
                    out.display()
                ),
                "грузятся конфигуратором: Конфигурация → Загрузить конфигурацию из файлов",
            )
            .hint(plat_err.why));
        }
    };

    let mut dst = dst.to_string();
    if !dst.to_lowercase().ends_with(".cfe") {
        dst.push_str(".cfe");
    }
    let dst_path = PathBuf::from(&dst);
    if let Some(dir) = dst_path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).map_err(|e| {
                Refusal::new(
                    Kind::Internal,
                    format!("каталог {} не создан", dir.display()),
                    e.to_string(),
                )
            })?;
        }
    }

    // Промежуточная база нужна потому, что .cfe рождается только выгрузкой ИЗ базы:
    // конфигуратор не умеет собирать расширение прямо из XML.
    //
    // Побайтового равенства с прежней сборкой ждать не надо, и это не наш дефект:
    // платформа проставляет внутрь `configVersion` — метку, меняющуюся от прогона
    // к прогону. Замер 04.09.2026: ДВА прогона одной и той же Go-версии подряд дали
    // разные файлы, при этом распакованное содержимое совпало побайтово (6 файлов,
    // модуль на 175 405 байт идентичен). Поэтому сверять собранное расширение надо
    // распаковкой, а не хешем файла.
    let tmp_ib = std::env::temp_dir().join(format!("gf-data-1c-ib-{}", std::process::id()));
    std::fs::create_dir_all(&tmp_ib)
        .map_err(|e| Refusal::new(Kind::Internal, "временная база не создана", e.to_string()))?;
    let _guard_ib = TempDir(tmp_ib.clone());

    let ibcmd = Path::new(&exe).with_file_name("ibcmd.exe");
    if !ibcmd.is_file() {
        return Err(Refusal::new(
            Kind::BadRequest,
            format!("рядом с {exe} нет ibcmd.exe"),
            "сборка .cfe невозможна",
        ));
    }

    let data = format!("--data={}", tmp_ib.display());
    let name = format!("--name={EXTENSION_NAME}");
    let ext = format!("--extension={EXTENSION_NAME}");
    let steps: Vec<Vec<String>> = vec![
        vec![
            "infobase".into(),
            "create".into(),
            data.clone(),
            "--create-database".into(),
        ],
        vec![
            "extension".into(),
            "create".into(),
            data.clone(),
            name,
            "--name-prefix=GT_".into(),
        ],
        vec![
            "config".into(),
            "import".into(),
            data.clone(),
            ext.clone(),
            src.to_string_lossy().to_string(),
        ],
        vec!["config".into(), "save".into(), data, ext, dst.clone()],
    ];
    for args in steps {
        let out = Command::new(&ibcmd).args(&args).output();
        let bad = match &out {
            Err(e) => Some((e.to_string(), Vec::new())),
            Ok(o) if !o.status.success() => Some((
                format!("код возврата {}", o.status),
                [o.stdout.clone(), o.stderr.clone()].concat(),
            )),
            Ok(_) => None,
        };
        if let Some((why, output)) = bad {
            let шаг = args[..2].join(" ");
            let mut r = Refusal::new(Kind::Internal, format!("шаг «{шаг}» не удался"), why);
            let текст = String::from_utf8_lossy(&output).trim().to_string();
            if !текст.is_empty() {
                r = r.hint(текст);
            }
            return Err(r);
        }
    }
    Ok(dst_path)
}

/// Удаляет временный каталог при выходе из области видимости — замена `defer os.RemoveAll`.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Где искать платформу.
///
/// Двух путей на диске C: не хватает: платформу ставят на другой диск, 64-битная версия
/// живёт в каталоге `1cv8x64`, а каталог установки 1С разрешает задать руками. Поэтому
/// корни собираются, а не перечисляются: явное указание через `GT_DATA_1C_PLATFORM`,
/// переменные окружения `ProgramFiles` (на 64-битной Windows их несколько) и оба имени
/// каталога. Порядок важен: сначала то, что назвал человек.
pub fn platform_roots() -> Vec<String> {
    let mut roots: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut add = |dir: String| {
        if dir.is_empty() || !seen.insert(dir.to_lowercase()) {
            return;
        }
        roots.push(dir);
    };

    if let Ok(env) = std::env::var("GT_DATA_1C_PLATFORM") {
        if !env.is_empty() {
            add(env);
        }
    }
    for base in [
        std::env::var("ProgramFiles").unwrap_or_default(),
        std::env::var("ProgramFiles(x86)").unwrap_or_default(),
        std::env::var("ProgramW6432").unwrap_or_default(),
        r"C:\Program Files".to_string(),
        r"C:\Program Files (x86)".to_string(),
    ] {
        if base.is_empty() {
            continue;
        }
        add(Path::new(&base).join("1cv8").to_string_lossy().to_string());
        add(Path::new(&base)
            .join("1cv8x64")
            .to_string_lossy()
            .to_string());
    }
    roots
}

/// Ищет конфигуратор: по указанному пути либо среди установленных версий, выбирая старшую.
pub fn resolve_platform(explicit: &str) -> Result<String, Refusal> {
    resolve_platform_in(explicit, &platform_roots())
}

/// То же, но корни поиска задаются явно: так поведение проверяется тестом на любой машине,
/// а не только там, где платформы нет.
pub fn resolve_platform_in(explicit: &str, roots: &[String]) -> Result<String, Refusal> {
    if !explicit.is_empty() {
        let p = Path::new(explicit);
        if p.is_file() {
            return Ok(explicit.to_string());
        }
        let candidate = p.join("1cv8.exe");
        if candidate.is_file() {
            return Ok(candidate.to_string_lossy().to_string());
        }
        return Err(Refusal::new(
            Kind::BadRequest,
            "конфигуратор не найден по указанному пути",
            explicit.to_string(),
        ));
    }

    let mut found: Vec<String> = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let exe = entry.path().join("bin").join("1cv8.exe");
            if exe.is_file() {
                found.push(exe.to_string_lossy().to_string());
            }
        }
    }
    if found.is_empty() {
        return Err(Refusal::new(
            Kind::BadRequest,
            "конфигуратор 1С (1cv8.exe) не найден",
            format!("искали в: {}", roots.join(", ")),
        )
        .hint(
            "платформа в другом месте — укажите путь флагом -platform, либо задайте каталог \
             версий в переменной GT_DATA_1C_PLATFORM",
        ));
    }

    // Версии сравниваются по числам, а не по строке — иначе 8.3.9 окажется «новее» 8.3.27.
    found.sort_by(|a, b| {
        if version_less(&version_of(b), &version_of(a)) {
            std::cmp::Ordering::Less
        } else if version_less(&version_of(a), &version_of(b)) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    Ok(found[0].clone())
}

/// Достаёт «8.3.27.2130» из пути вида `...\1cv8\8.3.27.2130\bin\1cv8.exe`.
pub fn version_of(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        if part.eq_ignore_ascii_case("bin") && i > 0 {
            return parts[i - 1].to_string();
        }
    }
    String::new()
}

/// Сравнивает версии почисленно.
pub fn version_less(a: &str, b: &str) -> bool {
    let (as_, bs): (Vec<&str>, Vec<&str>) = (a.split('.').collect(), b.split('.').collect());
    for i in 0..as_.len().min(bs.len()) {
        let (x, y) = (atoi(as_[i]), atoi(bs[i]));
        if x != y {
            return x < y;
        }
    }
    as_.len() < bs.len()
}

/// Число из начала строки: «2130» → 2130, «8x» → 8, «x8» → 0.
///
/// Своя, а не `parse`: разбор обязан не падать на мусоре, а брать ведущие цифры — так же,
/// как в Go-версии, откуда правило и перенесено.
pub fn atoi(s: &str) -> i64 {
    let mut n = 0i64;
    for r in s.chars() {
        if !r.is_ascii_digit() {
            return n;
        }
        n = n * 10 + (r as i64 - '0' as i64);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn версия_достаётся_из_пути() {
        for (путь, ждём) in [
            (
                r"C:\Program Files\1cv8\8.3.27.2130\bin\1cv8.exe",
                "8.3.27.2130",
            ),
            (
                "C:/Program Files/1cv8x64/8.3.9.1818/bin/1cv8.exe",
                "8.3.9.1818",
            ),
            ("/opt/1cv8/8.3.20.1674/bin/1cv8.exe", "8.3.20.1674"),
            ("нет-версии/1cv8.exe", ""),
            ("", ""),
        ] {
            assert_eq!(version_of(путь), ждём, "version_of({путь:?})");
        }
    }

    #[test]
    fn версии_сравниваются_по_числам_а_не_по_строке() {
        // Иначе 8.3.9 окажется «новее» 8.3.27 — строковое сравнение ставит «9» после «2».
        assert!(version_less("8.3.9", "8.3.27"));
        assert!(!version_less("8.3.27", "8.3.9"));
        assert!(
            !version_less("8.3.27.2130", "8.3.27.2130"),
            "равные не меньше"
        );
        // Более длинная версия старше при равных общих частях.
        assert!(version_less("8.3.27", "8.3.27.2130"));
        assert!(!version_less("8.3.27.2130", "8.3.27"));
        // Мусор не роняет разбор: нечисловое даёт ноль и проигрывает.
        assert!(version_less("", "8.3.1"));
        assert!(version_less("8.3.x", "8.3.1"));
    }

    #[test]
    fn число_берётся_из_начала_строки() {
        // Своя реализация, а не parse: разбор обязан не падать на мусоре.
        assert_eq!(atoi("2130"), 2130);
        assert_eq!(atoi("8x"), 8);
        assert_eq!(atoi(""), 0);
        assert_eq!(atoi("x8"), 0, "нечисловое начало — ноль");
        assert_eq!(atoi("007"), 7);
    }

    #[test]
    fn явный_путь_к_файлу_принимается_как_есть() {
        let exe = std::env::current_exe().expect("свой путь");
        let got = resolve_platform_in(&exe.to_string_lossy(), &[]).expect("файл существует");
        assert_eq!(got, exe.to_string_lossy());
    }

    #[test]
    fn несуществующий_явный_путь_даёт_отказ() {
        let нет = std::env::temp_dir().join("нет-такого-каталога-12345");
        let err = resolve_platform_in(&нет.to_string_lossy(), &[]).unwrap_err();
        assert_eq!(err.kind, Kind::BadRequest);
        assert!(
            err.to_string().contains("не найден по указанному пути"),
            "{err}"
        );
    }

    #[test]
    fn отказ_называет_места_поиска() {
        // Без перечня мест отказ отправляет искать вслепую.
        let err = resolve_platform_in(
            "",
            &[r"C:\нет\такого".to_string(), r"D:\и\такого".to_string()],
        )
        .unwrap_err();
        let текст = err.to_string();
        for кусок in [
            "1cv8.exe",
            r"C:\нет\такого",
            r"D:\и\такого",
            "-platform",
            "GT_DATA_1C_PLATFORM",
        ] {
            assert!(текст.contains(кусок), "в отказе нет {кусок:?}:\n{текст}");
        }
    }

    #[test]
    fn выбирается_старшая_версия() {
        let root = std::env::temp_dir().join(format!("gf-plat-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for v in ["8.3.9.1818", "8.3.27.2130", "8.3.20.1674"] {
            let bin = root.join(v).join("bin");
            std::fs::create_dir_all(&bin).expect("каталог");
            std::fs::write(bin.join("1cv8.exe"), b"x").expect("файл");
        }
        let got = resolve_platform_in("", &[root.to_string_lossy().to_string()]).expect("найдено");
        assert_eq!(
            version_of(&got),
            "8.3.27.2130",
            "8.3.9 не должна выиграть у 8.3.27"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn корни_поиска_без_повторов() {
        // На 64-битной Windows ProgramFiles и ProgramW6432 совпадают — дубль не нужен.
        let roots = platform_roots();
        let mut seen = std::collections::HashSet::new();
        for r in &roots {
            assert!(seen.insert(r.to_lowercase()), "повтор корня: {r}");
        }
        assert!(
            roots.iter().any(|r| r.ends_with("1cv8")),
            "среди корней нет 1cv8: {roots:?}"
        );
        assert!(
            roots.iter().any(|r| r.ends_with("1cv8x64")),
            "64-битный каталог не ищется: {roots:?}"
        );
    }

    #[test]
    fn база_обязательна_для_установки() {
        let err = install(&Options {
            base: "   ".into(),
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(err.kind, Kind::BadRequest);
        assert!(err.to_string().contains("база не названа"), "{err}");
    }

    #[test]
    fn цель_обязательна_для_выгрузки() {
        let err = export("", "").unwrap_err();
        assert_eq!(err.kind, Kind::BadRequest);
        assert!(err.to_string().contains("куда выгружать"), "{err}");
    }

    #[test]
    fn расширение_встроено_в_эту_сборку() {
        // Проверка ловит сборку без шага build-extension.ps1: без неё беда вскрывалась бы
        // отказом конфигуратора про «принадлежность основного объекта конфигурации» —
        // сообщением, которое отправляет искать причину не туда.
        assert!(
            extension_built(),
            "расширение не встроено. {EXTENSION_NOT_BUILT}"
        );
    }

    #[test]
    fn отказ_без_расширения_называет_починку() {
        // Текст отказа обязан назвать команду сборки, а не просто «нет файла».
        assert!(EXTENSION_NOT_BUILT.contains("build-extension.ps1"));
        assert!(EXTENSION_NOT_BUILT.contains("cargo build"));
    }
}
