//! Данные базы: проверка запроса, его выполнение, счёт записей, итоги регистра.
//!
//! Здесь работает гейт построителя: `query` выполняет только тот текст, который в этой же
//! сессии собрал `query_build`. Механизм, а не правило — см. [`super::gate`].

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::refusal::{Kind, Refusal};

use super::lenient::StringList;
use super::queryhints::{enrich_query_refusal, quiet_traps};
use super::Set;

// ── Проверка запроса ─────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct QueryCheckInput {
    /// Имя базы 1С из реестра. Обязательно.
    pub base: String,
    /// Текст запроса на языке запросов 1С для проверки.
    pub query: String,
}

pub const QUERY_CHECK_NAME: &str = "query_check";
pub const QUERY_CHECK_DESCRIPTION: &str = "Проверить синтаксис запроса 1С без выполнения — \
найдёт ошибки в ВЫБРАТЬ/SELECT и покажет, какие колонки вернёт запрос. Вызывай перед query: \
разбор не обращается к данным и стоит несоизмеримо дешевле самого запроса.";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CheckReply {
    #[serde(rename = "колонки")]
    columns: Vec<String>,
}

// ── Запрос ───────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct QueryInput {
    pub base: String,
    /// Текст запроса. Только ВЫБРАТЬ/SELECT. Параметры через `&ИмяПараметра`.
    pub query: String,
    /// Параметры запроса: ключ без амперсанда. Даты строкой ГГГГ-ММ-ДД.
    pub parameters: BTreeMap<String, Value>,
    /// Максимум строк результата (по умолчанию 100, максимум 1000).
    pub limit: i64,
    /// Пропустить столько строк — следующая страница.
    pub offset: i64,
}

pub const QUERY_NAME: &str = "query";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct QueryReply {
    #[serde(rename = "колонки")]
    pub(super) columns: Vec<String>,
    #[serde(rename = "строк")]
    pub(super) rows_shown: i64,
    #[serde(rename = "всегоСтрок")]
    pub(super) rows_total: i64,
    #[serde(rename = "смещение")]
    pub(super) offset: i64,
    #[serde(rename = "следующееСмещение")]
    pub(super) next_offset: i64,
    #[serde(rename = "естьЕщё")]
    pub(super) has_more: bool,
    #[serde(rename = "обрезано")]
    pub(super) truncated: bool,
    #[serde(rename = "строки")]
    pub(super) rows: Vec<Map<String, Value>>,
}

// ── Счёт записей ─────────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct CountInput {
    pub base: String,
    /// Таблица 1С: Справочник.X, Документ.X, РегистрНакопления.X и т.д.
    pub table: String,
    /// Условие отбора без слова ГДЕ.
    pub where_: String,
    pub parameters: BTreeMap<String, Value>,
}

pub const COUNT_NAME: &str = "count";
pub const COUNT_DESCRIPTION: &str = "Посчитать записи таблицы 1С — есть ли вообще данные, \
сколько документов за период, сколько элементов справочника. Отбор задаётся условием where \
с параметрами &Имя. Дешевле и надёжнее полного запроса: текст собирается сервером, поэтому \
счёт нельзя сочинить.";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CountReply {
    #[serde(rename = "таблица")]
    table: String,
    #[serde(rename = "отбор")]
    filter: String,
    #[serde(rename = "записей")]
    records: i64,
}

// ── Итоги регистра ───────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct RegisterInput {
    pub base: String,
    /// Имя регистра накопления без префикса, например ТоварыНаСкладах.
    pub register: String,
    /// Какая виртуальная таблица нужна: Остатки (умолчание), Обороты, ОстаткиИОбороты.
    pub kind: String,
    /// Дата остатков для `kind=Остатки`. Пусто — на текущий момент.
    pub period: String,
    pub start: String,
    pub end: String,
    /// Измерения-разрезы для группировки. Пусто — итог одной строкой.
    pub dimensions: StringList,
    /// Ресурсы виртуальной таблицы. Имена берутся из `object`.
    ///
    /// Поле намеренно необязательное: пустые `resources` — частая ошибка, и отвечать на
    /// неё должен наш отказ с подсказкой, где взять имена, а не сухое «missing properties».
    pub resources: StringList,
    pub where_: String,
    pub parameters: BTreeMap<String, Value>,
    pub limit: i64,
}

pub const REGISTER_NAME: &str = "register";

impl Set {
    /// Проверяет синтаксис запроса без выполнения.
    pub fn query_check(&self, input: &QueryCheckInput) -> Result<String, Refusal> {
        if input.query.trim().is_empty() {
            return Err(Refusal::new(
                Kind::BadRequest,
                "текст запроса пуст",
                "поле query обязательно",
            ));
        }
        let client = self.channel_for(&input.base)?;

        // Гейт: после отказа проверки следующий текст не разбирается, пока не позван
        // построитель. Стоит ПОСЛЕ проверки базы: отказ «база не названа» важнее и не
        // должен подменяться.
        if !self.allow_raw_query {
            let (ok, hint) = self.gate.check_allowed();
            if !ok {
                return Err(Refusal::new(
                    Kind::BadRequest,
                    "проверка текста закрыта после отказа",
                    hint,
                ));
            }
        }

        let reply: CheckReply = client
            .tell("check", &json!({ "query": input.query }))
            .map_err(|e| {
                self.gate.on_check_refused(&input.query);
                // Проверка запроса — то место, где подсказка нужнее всего: сюда приходят
                // с черновиком. К отказу платформы дописан ход: следующий текст не
                // примут, идти в построитель. Подсказку строит сам гейт — под источник
                // отказанного текста.
                let params = BTreeMap::new();
                let enriched = enrich_query_refusal(e, &input.query, &params);
                let (_, hint) = self.gate.check_allowed();
                enriched.hint(hint)
            })?;

        self.gate.on_check_passed();

        // Разбор — не пропуск: выполнить можно только текст построителя.
        let mut out = format!(
            "Запрос разобран. Колонки ({}): {}",
            reply.columns.len(),
            reply.columns.join(", ")
        );
        out.push_str(
            "\nВыполнить этот текст query нельзя — выполняется только собранный query_build. \
             Проверка нужна для диагностики синтаксиса.",
        );

        // Разбор проверяет синтаксис, а не смысл. Конструкции, которые выполняются и молча
        // отдают не то, называются здесь: другого места у них нет — отказа платформа не
        // даёт, а пустая выборка неотличима от честного «данных нет».
        let traps = quiet_traps(&input.query);
        if !traps.is_empty() {
            out.push_str(
                "\n\nРазбор прошёл, но запрос содержит то, о чём платформа не предупредит:",
            );
            for t in traps {
                out.push_str(&format!("\n  ⚠ {t}"));
            }
        }
        Ok(out)
    }

    /// Выполняет запрос.
    pub fn query(&self, input: &QueryInput) -> Result<String, Refusal> {
        if input.query.trim().is_empty() {
            return Err(Refusal::new(
                Kind::BadRequest,
                "текст запроса пуст",
                "поле query обязательно",
            ));
        }
        let client = self.channel_for(&input.base)?;

        // Выполняется только текст, который в этой сессии собрал query_build. Написанный
        // руками не выполняется вовсе. После проверки базы: см. query_check.
        if !self.allow_raw_query && !self.gate.is_approved(&input.query) {
            return Err(Refusal::new(
                Kind::BadRequest,
                "текст запроса не собран построителем",
                "выполняется только текст, который в этой сессии вернул query_build — дословно; \
                 написанный руками текст не выполняется, даже разобранный query_check",
            )
            .hint(
                "соберите запрос query_build (источник, поля, отбор, группировка, порядок) и \
                 выполните его текст как есть",
            )
            .hint(
                "соединения и пакеты построитель не собирает — такой вопрос возвращается как \
                 «не собирается», без обходного текста",
            ));
        }

        let mut payload = Map::new();
        payload.insert("query".into(), json!(input.query));
        if !input.parameters.is_empty() {
            payload.insert("parameters".into(), json!(input.parameters));
        }
        if input.limit > 0 {
            payload.insert("limit".into(), json!(input.limit));
        }
        if input.offset > 0 {
            payload.insert("offset".into(), json!(input.offset));
        }

        let reply: QueryReply = client.tell("query", &Value::Object(payload)).map_err(|e| {
            // Отказ платформы точен, но односложен: к нему дописывается то, что известно
            // про виртуальные таблицы и язык запросов, — иначе вызывающий уходит угадывать.
            let params: BTreeMap<String, String> = input
                .parameters
                .keys()
                .map(|k| (k.clone(), String::new()))
                .collect();
            enrich_query_refusal(e, &input.query, &params)
        })?;

        Ok(render_table(&client.base().name, &reply))
    }

    /// Считает записи таблицы.
    pub fn count(&self, input: &CountInput) -> Result<String, Refusal> {
        if input.table.trim().is_empty() {
            return Err(Refusal::new(
                Kind::BadRequest,
                "таблица не названа",
                "поле table обязательно",
            ));
        }
        let client = self.channel_for(&input.base)?;

        let mut payload = Map::new();
        payload.insert("table".into(), json!(input.table));
        if !input.where_.is_empty() {
            payload.insert("where".into(), json!(input.where_));
        }
        if !input.parameters.is_empty() {
            payload.insert("parameters".into(), json!(input.parameters));
        }

        let reply: CountReply = client.tell("count", &Value::Object(payload))?;

        let mut out = format!(
            "{}, база {}: записей {}",
            reply.table,
            client.base().name,
            reply.records
        );
        if !reply.filter.is_empty() {
            out.push_str(&format!("\nОтбор: {}", reply.filter));
        }
        Ok(out)
    }

    /// Итоги регистра накопления через виртуальные таблицы.
    pub fn register(&self, input: &RegisterInput) -> Result<String, Refusal> {
        if input.register.trim().is_empty() {
            return Err(Refusal::new(
                Kind::BadRequest,
                "регистр не назван",
                "поле register обязательно",
            ));
        }
        let client = self.channel_for(&input.base)?;

        let mut payload = Map::new();
        payload.insert("register".into(), json!(input.register));
        for (key, value) in [
            ("kind", &input.kind),
            ("period", &input.period),
            ("start", &input.start),
            ("end", &input.end),
            ("where", &input.where_),
        ] {
            if !value.trim().is_empty() {
                payload.insert(key.into(), json!(value));
            }
        }
        if !input.dimensions.is_empty() {
            payload.insert("dimensions".into(), json!(input.dimensions.as_slice()));
        }
        if !input.resources.is_empty() {
            payload.insert("resources".into(), json!(input.resources.as_slice()));
        }
        if !input.parameters.is_empty() {
            payload.insert("parameters".into(), json!(input.parameters));
        }
        if input.limit > 0 {
            payload.insert("limit".into(), json!(input.limit));
        }

        let reply: QueryReply = client.tell("register", &Value::Object(payload))?;
        Ok(render_table(&client.base().name, &reply))
    }
}

/// Печатает результат запроса.
pub(super) fn render_table(base: &str, reply: &QueryReply) -> String {
    let mut out = format!("База {base}: строк {}", reply.rows_shown);
    if reply.offset > 0 {
        out.push_str(&format!(", начиная с {}", reply.offset));
    }
    if reply.has_more {
        out.push_str(&format!(" из {} — показана часть", reply.rows_total));
    }
    out.push_str("\n\n");

    if reply.rows.is_empty() {
        out.push_str(
            "Результат пуст. Это ответ базы, а не отказ канала: запрос выполнен и вернул ноль \
             строк.",
        );
        return out;
    }

    for (i, row) in reply.rows.iter().enumerate() {
        out.push_str(&format!("{}.", i + 1));
        for col in &reply.columns {
            out.push_str(&format!(
                "  {col} = {}",
                render_value(row.get(col).unwrap_or(&Value::Null))
            ));
        }
        out.push('\n');
    }
    if reply.has_more {
        out.push_str(&format!(
            "\nПоказано {} из {} строк. Следующая страница: offset={} (для устойчивой разбивки \
             запрос должен содержать УПОРЯДОЧИТЬ). Весь результат целиком — инструмент export.",
            reply.rows_shown, reply.rows_total, reply.next_offset
        ));
    }
    let _ = reply.truncated;
    out
}

/// Печатает значение ячейки. Ссылка разворачивается в «представление (тип, идентификатор)»:
/// одного представления мало, по нему нельзя отобрать.
pub(super) fn render_value(value: &Value) -> String {
    match value {
        Value::Null => "—".to_string(),
        Value::String(s) => s.clone(),
        Value::Bool(true) => "да".to_string(),
        Value::Bool(false) => "нет".to_string(),
        Value::Number(n) => n.to_string(),
        Value::Object(m) => {
            let представление = m
                .get("представление")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let тип = m.get("тип").and_then(Value::as_str).unwrap_or_default();
            let идентификатор = m
                .get("идентификатор")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if тип.is_empty() {
                представление.to_string()
            } else {
                format!("{представление} ({тип}, {идентификатор})")
            }
        }
        other => other.to_string(),
    }
}

// ── Журнал регистрации ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EventLogInput {
    /// Имя базы 1С из реестра.
    pub base: String,
    /// Начало периода в формате ISO 8601, например 2026-03-01T00:00:00.
    pub start_date: String,
    /// Конец периода в формате ISO 8601.
    pub end_date: String,
    /// Уровень важности: Ошибка, Предупреждение, Информация, Примечание.
    pub level: String,
    /// Имя пользователя 1С для фильтрации.
    pub user: String,
    /// Максимум записей (по умолчанию 50, максимум 500).
    pub limit: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct EventLogRecord {
    #[serde(rename = "дата")]
    date: String,
    #[serde(rename = "уровень")]
    level: String,
    #[serde(rename = "пользователь")]
    user: String,
    #[serde(rename = "событие")]
    event: String,
    #[serde(rename = "комментарий")]
    comment: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct EventLogReply {
    #[serde(rename = "записей")]
    count: i64,
    #[serde(rename = "предел")]
    limit: i64,
    #[serde(rename = "записи")]
    records: Vec<EventLogRecord>,
}

/// Первая строка текста, обрезанная до разумной длины.
///
/// Комментарий записи журнала бывает в килобайт и с переводами строк: в перечне нужна
/// опознавательная строка, а не всё содержимое. Длина считается в СИМВОЛАХ (в Go —
/// в байтах): срез по середине кириллической буквы здесь паникует, а не портит вывод молча.
fn first_line(s: &str) -> String {
    let s = match s.find(['\r', '\n']) {
        Some(i) => &s[..i],
        None => s,
    };
    if s.chars().count() > 200 {
        let head: String = s.chars().take(200).collect();
        return format!("{head}…");
    }
    s.to_string()
}

impl Set {
    /// Читает журнал регистрации базы.
    pub fn event_log(&self, input: &EventLogInput) -> Result<String, Refusal> {
        let client = self.channel_for(&input.base)?;

        let limit = input.limit.to_string();
        let mut query: Vec<(&str, &str)> = Vec::new();
        for (key, value) in [
            ("start", input.start_date.trim()),
            ("end", input.end_date.trim()),
            ("level", input.level.trim()),
            ("user", input.user.trim()),
        ] {
            if !value.is_empty() {
                query.push((key, value));
            }
        }
        if input.limit > 0 {
            query.push(("limit", &limit));
        }

        let reply: EventLogReply = client.ask("eventlog", &query)?;

        let mut b = format!(
            "Журнал регистрации базы {}: записей {} (предел {})\n\n",
            client.base().name,
            reply.count,
            reply.limit
        );
        for rec in &reply.records {
            b.push_str(&format!(
                "{}  {:<14} {:<16} {}\n",
                rec.date, rec.level, rec.user, rec.event
            ));
            if !rec.comment.trim().is_empty() {
                b.push_str(&format!("    {}\n", first_line(&rec.comment)));
            }
        }
        // Упор в предел — не «данных больше нет»: молчаливое усечение и есть та ложь,
        // из-за которой пустая выдача читается как факт о базе.
        if reply.count == reply.limit {
            b.push_str(
                "\nВыдача упёрлась в предел: записей может быть больше — сузьте период \
                 или поднимите limit.",
            );
        }
        Ok(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_set(tag: &str) -> Set {
        let dir = std::env::temp_dir().join(format!("gfdata-data-{}-{tag}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("bases.json");
        let _ = std::fs::remove_file(&path);
        let mut s = Set::new("0.1.0");
        s.registry_path = Some(path);
        s
    }

    #[test]
    fn рукописный_текст_не_выполняется() {
        let s = make_set("гейт");
        let err = s
            .query(&QueryInput {
                base: "ut11".into(),
                query: "ВЫБРАТЬ Ссылка ИЗ Справочник.Номенклатура".into(),
                ..Default::default()
            })
            .unwrap_err();
        // База не заведена, поэтому первым отвечает отказ по базе — и это верный порядок:
        // «база не названа» важнее и не должен подменяться гейтом.
        assert_eq!(err.kind, Kind::UnknownBase);
    }

    #[test]
    fn гейт_срабатывает_после_того_как_база_разрешена() {
        let s = make_set("гейт-после-базы");
        {
            let mut reg = s.registry().unwrap();
            reg.add(crate::registry::Base {
                name: "ut11".into(),
                url: "http://127.0.0.1:9/hs/gt-data".into(),
                ..Default::default()
            })
            .unwrap();
        }
        let err = s
            .query(&QueryInput {
                base: "ut11".into(),
                query: "ВЫБРАТЬ Ссылка ИЗ Справочник.Номенклатура".into(),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(err.kind, Kind::BadRequest);
        assert!(
            err.to_string().contains("не собран построителем"),
            "гейт обязан сработать ДО обращения к базе: {err}"
        );
    }

    #[test]
    fn одобренный_построителем_текст_проходит_гейт() {
        let s = make_set("одобренный");
        {
            let mut reg = s.registry().unwrap();
            reg.add(crate::registry::Base {
                name: "ut11".into(),
                url: "http://127.0.0.1:9/hs/gt-data".into(),
                ..Default::default()
            })
            .unwrap();
        }
        let текст = "ВЫБРАТЬ Ссылка ИЗ Справочник.Номенклатура";
        s.gate.approve(текст);
        let err = s
            .query(&QueryInput {
                base: "ut11".into(),
                query: текст.into(),
                ..Default::default()
            })
            .unwrap_err();
        // Гейт пройден — дальше упирается уже в мёртвый канал, и это другой отказ.
        assert_eq!(err.kind, Kind::NoWebServer, "{err}");
    }

    #[test]
    fn пустой_текст_отвергается_до_базы() {
        let s = make_set("пустой");
        let err = s.query(&QueryInput::default()).unwrap_err();
        assert_eq!(err.kind, Kind::BadRequest);
        assert!(err.to_string().contains("текст запроса пуст"), "{err}");
    }

    #[test]
    fn таблица_обязательна_для_счёта() {
        let s = make_set("счёт");
        let err = s.count(&CountInput::default()).unwrap_err();
        assert!(err.to_string().contains("таблица не названа"), "{err}");
    }

    #[test]
    fn регистр_обязателен() {
        let s = make_set("регистр");
        let err = s.register(&RegisterInput::default()).unwrap_err();
        assert!(err.to_string().contains("регистр не назван"), "{err}");
    }

    #[test]
    fn пустой_результат_это_ответ_базы_а_не_отказ() {
        let out = render_table(
            "ut11",
            &QueryReply {
                columns: vec!["Ссылка".into()],
                ..Default::default()
            },
        );
        assert!(
            out.contains("Это ответ базы, а не отказ канала"),
            "иначе пустота читается как поломка: {out}"
        );
    }

    #[test]
    fn ссылка_печатается_с_типом_и_идентификатором() {
        let v = json!({
            "представление": "Гвозди",
            "тип": "CatalogRef.Номенклатура",
            "идентификатор": "abc-123"
        });
        assert_eq!(
            render_value(&v),
            "Гвозди (CatalogRef.Номенклатура, abc-123)",
            "по одному представлению нельзя отобрать"
        );
    }

    #[test]
    fn пустая_ячейка_печатается_прочерком() {
        assert_eq!(render_value(&Value::Null), "—");
    }

    #[test]
    fn булево_печатается_по_русски() {
        assert_eq!(render_value(&json!(true)), "да");
        assert_eq!(render_value(&json!(false)), "нет");
    }

    #[test]
    fn целое_число_печатается_без_дробной_части() {
        assert_eq!(render_value(&json!(42)), "42");
        assert_eq!(render_value(&json!(42.5)), "42.5");
    }
    #[test]
    fn первая_строка_обрезается_по_символам() {
        // Комментарий записи журнала бывает в килобайт и с переводами строк: в перечне
        // нужна опознавательная строка. Обрезка по СИМВОЛАМ, а не по байтам: в Go срез
        // по середине кириллической буквы портит вывод молча, здесь он паникует.
        assert_eq!(first_line("одна строка"), "одна строка");
        assert_eq!(first_line("первая\nвторая"), "первая");
        assert_eq!(first_line("первая\r\nвторая"), "первая");
        let длинный = "я".repeat(300);
        let out = first_line(&длинный);
        assert_eq!(out.chars().count(), 201, "200 символов и многоточие");
        assert!(out.ends_with('…'));
        assert!(
            out.trim_end_matches('…').chars().all(|c| c == 'я'),
            "буква разрезана пополам"
        );
    }

    #[test]
    fn база_обязательна_для_журнала() {
        let err = make_set("журнал")
            .event_log(&EventLogInput::default())
            .unwrap_err();
        assert_eq!(err.kind, Kind::BadRequest);
    }
}
