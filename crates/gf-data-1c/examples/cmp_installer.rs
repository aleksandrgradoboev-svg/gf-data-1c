//! Установщик расширения — сверка с Go.
//!
//! Живую установку в базу пример не делает: она меняет конфигурацию и требует погашенного
//! веб-сервера. Сверяется всё, что до неё, — поиск платформы, выбор старшей версии, разбор
//! версии из пути, тексты отказов и состав распакованного расширения.
use gf_data_1c::installer::*;

fn main() {
    println!("=== versionOf ===");
    for p in [
        r"C:\Program Files\1cv8\8.3.27.2130\bin\1cv8.exe",
        "C:/Program Files/1cv8x64/8.3.9.1818/bin/1cv8.exe",
        "/opt/1cv8/8.3.20.1674/bin/1cv8.exe",
        "нет-версии/1cv8.exe",
        "",
    ] {
        println!("  {p:?} -> {:?}", version_of(p));
    }

    println!("=== versionLess ===");
    for (a, b) in [
        ("8.3.9", "8.3.27"),
        ("8.3.27", "8.3.9"),
        ("8.3.27.2130", "8.3.27.2130"),
        ("8.3.27", "8.3.27.2130"),
        ("8.3.27.2130", "8.3.27"),
        ("", "8.3.1"),
        ("8.3.x", "8.3.1"),
    ] {
        println!("  versionLess({a:?}, {b:?}) = {}", version_less(a, b));
    }

    println!("=== atoi ===");
    for s in ["2130", "8x", "", "x8", "007"] {
        println!("  atoi({s:?}) = {}", atoi(s));
    }

    println!("=== resolvePlatformIn: явный путь ===");
    let self_exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let got = resolve_platform_in(&self_exe, &[]);
    println!(
        "  явный файл: ok={} (совпал={})",
        got.is_ok(),
        got.as_deref().unwrap_or("") == self_exe
    );

    let нет = std::env::temp_dir().join("нет-такого-каталога-12345");
    match resolve_platform_in(&нет.to_string_lossy(), &[]) {
        Ok(p) => println!("  несуществующий путь: {p:?} (ошибки нет)"),
        Err(e) => println!("  несуществующий путь: {}", одной_строкой(&e.to_string())),
    }

    println!("=== resolvePlatformIn: пустые корни ===");
    if let Err(e) = resolve_platform_in("", &[]) {
        println!("  {}", одной_строкой(&e.to_string()));
    }

    println!("=== resolvePlatformIn: корни без платформы ===");
    if let Err(e) = resolve_platform_in(
        "",
        &[r"C:\нет\такого".to_string(), r"D:\и\такого".to_string()],
    ) {
        println!("  {}", одной_строкой(&e.to_string()));
    }

    println!("=== resolvePlatformIn: выбор старшей версии ===");
    // Раскладка из трёх версий; старшая обязана победить, и по числам, а не по строке.
    let root = std::env::temp_dir().join(format!("gf-plat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for v in ["8.3.9.1818", "8.3.27.2130", "8.3.20.1674"] {
        let bin = root.join(v).join("bin");
        let _ = std::fs::create_dir_all(&bin);
        let _ = std::fs::write(bin.join("1cv8.exe"), b"x");
    }
    match resolve_platform_in("", &[root.to_string_lossy().to_string()]) {
        Ok(p) => println!("  выбрана версия: {:?}", version_of(&p)),
        Err(e) => println!("  ОТКАЗ: {}", одной_строкой(&e.to_string())),
    }
    let _ = std::fs::remove_dir_all(&root);

    println!("=== platformRoots: состав ===");
    let roots = platform_roots();
    println!("  корней: {}", roots.len());
    for r in &roots {
        // Печатаем только хвост: полные пути машинозависимы.
        let p = std::path::Path::new(r);
        let база = p
            .parent()
            .and_then(|d| d.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let имя = p
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        println!("    …{база}/{имя}");
    }

    println!("=== extensionBuilt ===");
    println!("  встроено: {}", extension_built());

    println!("=== unpack: состав распакованного ===");
    match export_для_сверки() {
        Ok(files) => {
            for f in files {
                println!("  {f}");
            }
        }
        Err(e) => println!("  ОТКАЗ: {}", одной_строкой(&e.to_string())),
    }

    println!("=== Install: база не названа ===");
    if let Err(e) = install(&Options {
        base: "   ".into(),
        ..Default::default()
    }) {
        println!("  {}", одной_строкой(&e.to_string()));
    }

    println!("=== Export: не сказано куда ===");
    if let Err(e) = export("", "") {
        println!("  {}", одной_строкой(&e.to_string()));
    }
}

/// Раскладывает расширение во временный каталог и возвращает перечень файлов.
///
/// Своя обёртка вместо `unpack`: та приватная, а состав встроенного нужно увидеть — это
/// он и есть то, что уедет в базу.
fn export_для_сверки() -> Result<Vec<String>, gf_data_1c::refusal::Refusal> {
    let dir = std::env::temp_dir().join(format!("gf-cmp-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // export без платформы кладёт XML-исходники и возвращает отказ с их путём — ровно
    // то, что нужно для перечня.
    let _ = export(
        &dir.to_string_lossy(),
        "заведомо-несуществующая-платформа-12345",
    );
    let mut files = Vec::new();
    собрать(&dir, &dir, &mut files);
    files.sort();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(files)
}

fn собрать(корень: &std::path::Path, дир: &std::path::Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(дир) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            собрать(корень, &p, out);
        } else if let Ok(rel) = p.strip_prefix(корень) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// Отказ печатается одной строкой: в Go тексты многострочные, и сравнивать удобнее так.
fn одной_строкой(s: &str) -> String {
    s.replace('\n', " | ")
}
