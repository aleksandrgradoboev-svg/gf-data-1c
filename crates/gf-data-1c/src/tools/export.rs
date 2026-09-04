//! Выгрузка результата запроса в файл: когда строк больше, чем помещается в ответ.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::refusal::{Kind, Refusal};

use super::data::QueryReply;
use super::Set;

/// Размер порции при выгрузке. Совпадает с потолком инструментов: больше одного запроса
/// база всё равно не отдаёт.
const PAGE_SIZE: i64 = 1000;

/// Предохранитель. Выгрузка в миллионы строк почти всегда означает забытый отбор, а не
/// намерение: лучше упереться в понятный отказ, чем полчаса писать файл, который никто
/// не откроет.
const MAX_EXPORT_ROWS: i64 = 500_000;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ExportInput {
    pub base: String,
    /// Текст запроса. Только ВЫБРАТЬ. Для устойчивой выгрузки добавьте УПОРЯДОЧИТЬ.
    pub query: String,
    pub parameters: BTreeMap<String, Value>,
    /// `csv` (умолчание) или `jsonl`.
    pub format: String,
    /// Куда положить файл. Пусто — каталог выгрузок в профиле пользователя.
    pub path: String,
    /// Потолок строк. Пусто — предохранитель на 500 000.
    pub max_rows: i64,
}

pub const NAME: &str = "export";

impl Set {
    /// Выгружает результат запроса в файл.
    pub fn export(&self, input: &ExportInput) -> Result<String, Refusal> {
        if input.query.trim().is_empty() {
            return Err(Refusal::new(
                Kind::BadRequest,
                "текст запроса пуст",
                "поле query обязательно",
            ));
        }
        let client = self.channel_for(&input.base)?;

        // Тот же гейт, что у query: выгрузка выполняет только текст построителя. Без
        // этого export — обход гейта в один ход: query рукописный текст отвергает, а
        // export с тем же текстом писал файл (найдено живой сессией 02.09.2026, модель
        // дыру нашла сама с четвёртой попытки).
        if !self.allow_raw_query && !self.gate.is_approved(&input.query) {
            return Err(Refusal::new(
                Kind::BadRequest,
                "текст запроса не собран построителем",
                "выгрузка выполняет только текст, который в этой сессии вернул query_build — \
                 дословно; написанный руками текст не выполняется, даже разобранный query_check",
            )
            .hint(
                "соберите запрос query_build (источник, поля, отбор, группировка, порядок) и \
                 выгружайте его текст как есть",
            ));
        }

        let format = input.format.trim().to_lowercase();
        let format = if format.is_empty() {
            "csv".to_string()
        } else {
            format
        };
        if format != "csv" && format != "jsonl" {
            return Err(Refusal::new(
                Kind::BadRequest,
                "формат не распознан",
                format!("format={}", input.format),
            )
            .hint("допустимо: csv, jsonl"));
        }

        let limit = if input.max_rows > 0 {
            input.max_rows
        } else {
            MAX_EXPORT_ROWS
        };

        let path = export_path(&input.path, &client.base().name, &format)?;
        let mut file = std::fs::File::create(&path)
            .map_err(|e| Refusal::new(Kind::Internal, "файл выгрузки не создан", e.to_string()))?;

        let started = std::time::Instant::now();
        let mut columns: Vec<String> = Vec::new();
        let mut written: i64 = 0;
        let mut offset: i64 = 0;
        // Присваивается в первой же итерации: цикл всегда делает хотя бы один запрос.
        let mut total_known: i64;

        loop {
            let mut payload = Map::new();
            payload.insert("query".into(), json!(input.query));
            payload.insert("limit".into(), json!(PAGE_SIZE));
            payload.insert("offset".into(), json!(offset));
            if !input.parameters.is_empty() {
                payload.insert("parameters".into(), json!(input.parameters));
            }

            let reply: QueryReply = match client.tell("query", &Value::Object(payload)) {
                Ok(r) => r,
                Err(e) => {
                    // Недописанный файл хуже отсутствующего: он выглядит результатом.
                    let _ = std::fs::remove_file(&path);
                    return Err(e);
                }
            };
            total_known = reply.rows_total;

            if written == 0 {
                columns = reply.columns.clone();
                if format == "csv" {
                    writeln!(file, "{}", csv_line(&columns)).map_err(|e| {
                        Refusal::new(Kind::Internal, "заголовок не записан", e.to_string())
                    })?;
                }
            }

            for row in &reply.rows {
                write_row(&mut file, &format, &columns, row)?;
                written += 1;
                if written >= limit {
                    break;
                }
            }

            if !reply.has_more || written >= limit || reply.rows.is_empty() {
                break;
            }
            offset = reply.next_offset;
        }

        file.flush()
            .map_err(|e| Refusal::new(Kind::Internal, "файл выгрузки не дописан", e.to_string()))?;

        let mut out = format!("Выгружено строк: {written}");
        if total_known > 0 {
            out.push_str(&format!(" из {total_known}"));
        }
        out.push_str(&format!(
            "\nФайл: {}\nФормат: {}, колонок {}, время {}ms",
            path.display(),
            format,
            columns.len(),
            started.elapsed().as_millis()
        ));
        if written >= limit && total_known > written {
            out.push_str(&format!(
                "\n⚠ Выгрузка остановлена предохранителем на {limit} строках, в результате их \
                 {total_known}. Это не весь результат: уточните отбор или поднимите max_rows."
            ));
        }
        Ok(out)
    }
}

/// Пишет строку в выбранном формате.
///
/// CSV разворачивает ссылку в представление: файл открывают глазами в таблице, и объект
/// с типом и идентификатором там только мешает. JSONL сохраняет всё — его читает программа.
fn write_row(
    file: &mut std::fs::File,
    format: &str,
    columns: &[String],
    row: &Map<String, Value>,
) -> Result<(), Refusal> {
    let line = if format == "csv" {
        let values: Vec<String> = columns
            .iter()
            .map(|c| csv_value(row.get(c).unwrap_or(&Value::Null)))
            .collect();
        csv_line(&values)
    } else {
        serde_json::to_string(row)
            .map_err(|e| Refusal::new(Kind::Internal, "строка не записана", e.to_string()))?
    };
    writeln!(file, "{line}")
        .map_err(|e| Refusal::new(Kind::Internal, "строка не записана", e.to_string()))
}

/// Собирает строку CSV. Разделитель — точка с запятой: Excel в русской локали ждёт
/// именно её, иначе весь файл ложится в одну колонку.
fn csv_line(values: &[String]) -> String {
    values
        .iter()
        .map(|v| csv_escape(v))
        .collect::<Vec<_>>()
        .join(";")
}

/// Экранирует значение по правилам CSV: кавычки удваиваются, поле берётся в кавычки,
/// если содержит разделитель, кавычку или перевод строки.
fn csv_escape(value: &str) -> String {
    if value.contains([';', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn csv_value(value: &Value) -> String {
    if let Value::Object(m) = value {
        return m
            .get("представление")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
    if value.is_null() {
        return String::new();
    }
    super::data::render_value(value)
}

/// Выбирает имя файла: заданное вызывающим либо своё, с базой и временем.
fn export_path(explicit: &str, base: &str, format: &str) -> Result<PathBuf, Refusal> {
    if !explicit.trim().is_empty() {
        let p = PathBuf::from(explicit);
        if let Some(dir) = p.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir).map_err(|e| {
                    Refusal::new(Kind::Internal, "каталог выгрузки не создан", e.to_string())
                })?;
            }
        }
        return Ok(p);
    }

    // Каталог прежний — см. registry::default_path.
    let dir = crate::journal::default_path()
        .parent()
        .map(|p| p.join("exports"))
        .unwrap_or_else(|| PathBuf::from("exports"));
    std::fs::create_dir_all(&dir)
        .map_err(|e| Refusal::new(Kind::Internal, "каталог выгрузок не создан", e.to_string()))?;

    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    Ok(dir.join(format!("{base}-{stamp}.{format}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_set(tag: &str) -> Set {
        let dir = std::env::temp_dir().join(format!("gfdata-export-{}-{tag}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("bases.json");
        let _ = std::fs::remove_file(&path);
        let mut s = Set::new("0.1.0");
        s.registry_path = Some(path);
        s
    }

    fn с_базой(tag: &str) -> Set {
        let s = make_set(tag);
        let mut reg = s.registry().unwrap();
        reg.add(crate::registry::Base {
            name: "ut11".into(),
            url: "http://127.0.0.1:9/hs/gt-data".into(),
            ..Default::default()
        })
        .unwrap();
        s
    }

    #[test]
    fn выгрузка_рукописного_текста_это_обход_гейта() {
        let s = с_базой("гейт");
        let err = s
            .export(&ExportInput {
                base: "ut11".into(),
                query: "ВЫБРАТЬ Ссылка ИЗ Справочник.Номенклатура".into(),
                ..Default::default()
            })
            .unwrap_err();
        assert!(
            err.to_string().contains("не собран построителем"),
            "иначе export — дыра в гейте в один ход: {err}"
        );
    }

    #[test]
    fn незнакомый_формат_отвергается_с_перечнем() {
        let s = с_базой("формат");
        s.gate.approve("ВЫБРАТЬ 1");
        let err = s
            .export(&ExportInput {
                base: "ut11".into(),
                query: "ВЫБРАТЬ 1".into(),
                format: "xlsx".into(),
                ..Default::default()
            })
            .unwrap_err();
        assert!(err.to_string().contains("допустимо: csv, jsonl"), "{err}");
    }

    #[test]
    fn разделитель_csv_точка_с_запятой() {
        let line = csv_line(&["Код".to_string(), "Наименование".to_string()]);
        assert_eq!(line, "Код;Наименование", "Excel в русской локали ждёт «;»");
    }

    #[test]
    fn значение_с_разделителем_берётся_в_кавычки() {
        assert_eq!(csv_escape("Гвозди; шурупы"), "\"Гвозди; шурупы\"");
        assert_eq!(csv_escape("Труба \"Д\""), "\"Труба \"\"Д\"\"\"");
        assert_eq!(csv_escape("простое"), "простое");
    }

    #[test]
    fn ссылка_в_csv_сворачивается_до_представления() {
        let v = json!({
            "представление": "Гвозди",
            "тип": "CatalogRef.Номенклатура",
            "идентификатор": "abc"
        });
        assert_eq!(
            csv_value(&v),
            "Гвозди",
            "файл открывают глазами: тип и идентификатор там мешают"
        );
    }

    #[test]
    fn пустая_ячейка_в_csv_пуста_а_не_прочерк() {
        assert_eq!(csv_value(&Value::Null), "");
    }

    #[test]
    fn имя_файла_по_умолчанию_несёт_базу_и_время() {
        let p = export_path("", "ut11", "csv").unwrap();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("ut11-"), "{name}");
        assert!(name.ends_with(".csv"), "{name}");
    }
}
