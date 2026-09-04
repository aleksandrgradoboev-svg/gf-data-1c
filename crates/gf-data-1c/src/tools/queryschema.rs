//! Разбор и сборка запросов 1С: `query_parse` и `query_build`.
//!
//! Ни то, ни другое сервер не делает сам: разбирает и собирает `СхемаЗапроса` внутри
//! базы, а здесь — маршалинг входа и печать ответа. Зато вход разбирается ЛОЯЛЬНО,
//! и это не украшение: построитель заведён ради того, чтобы слабая модель не сочиняла
//! текст запроса руками, а строгая схема выгоняет её обратно к сочинению одним отказом
//! валидатора, которого модель не понимает.

use std::collections::BTreeMap;

use serde::de::{Deserialize, Deserializer};
use serde_json::{json, Map, Value};

use crate::refusal::{Kind, Refusal};

use super::queryhints::enrich_query_refusal;
use super::Set;

// ── Разбор запроса в структуру ────────────────────────────────────────────────

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct QueryParseInput {
    /// Имя базы 1С из реестра.
    pub base: String,
    /// Текст запроса, который нужно разобрать.
    pub query: String,
}

/// Состав ответа расширения. Поле «запросы» остаётся сырым JSON: его структура
/// рекурсивна (вложенные запросы), и пересобирать её в типы — значит молча потерять
/// то, чего в типах не предусмотрели.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct ParseReply {
    #[serde(rename = "запросовВПакете")]
    queries_in_batch: i64,
    #[serde(rename = "таблицы")]
    tables: Vec<String>,
    #[serde(rename = "временныеТаблицы")]
    temp_tables: Vec<String>,
    #[serde(rename = "параметры")]
    params: Vec<String>,
    #[serde(rename = "запросы")]
    queries: Value,
}

/// Перепечатывает сырой JSON с отступами. Расширение отдаёт его одной строкой,
/// а читать структуру пакета предстоит модели — ей отступы и нужны.
fn indent_json(raw: &Value) -> String {
    serde_json::to_string_pretty(raw).unwrap_or_else(|_| raw.to_string())
}

/// Пересобирает объект с ключами по алфавиту, рекурсивно.
///
/// `serde_json` собран с `preserve_order` — порядок полей базы нужен разбору запроса,
/// где структура печатается модели «как отдала платформа». Но вычисленные значения
/// параметров модель КОПИРУЕТ в следующий вызов, и там порядок обязан быть одним и тем
/// же от раза к разу: плавающий порядок ключей в скопированном куске — это разный текст
/// на одинаковых данных. Go сортирует ключи map при сериализации сам, здесь — явно.
fn sorted_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted: Vec<(&String, &Value)> = map.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(b.0));
            Value::Object(
                sorted
                    .into_iter()
                    .map(|(k, v)| (k.clone(), sorted_json(v)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted_json).collect()),
        other => other.clone(),
    }
}

// ── Сборка запроса из структуры ───────────────────────────────────────────────

/// Колонка результата.
///
/// Ключи схемы — ЛАТИНСКИЕ, значения остаются русскими (это язык запросов 1С).
/// Прежняя схема смешивала латинский `base` с русскими `источник`, `поля`, `отбор`,
/// и модели приходилось угадывать язык КАЖДОГО ключа. Замер по журналу Kilo
/// (31.08.2026): слабая модель написала «база» вместо `base`, а «выражение» —
/// китайским иероглифом; оба вызова отвергнуты валидацией, и модель ушла в 15 оборотов
/// ручного сочинения запроса — от того самого инструмента, который его отменяет.
/// Русские имена приняты алиасами при разборе, чтобы прежние вызовы не сломались.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryBuildField {
    /// Имя поля источника, можно через точку.
    pub field: String,
    /// Функция над полем: СУММА, КОЛИЧЕСТВО, НАЧАЛОПЕРИОДА…
    pub func: String,
    /// Период для НАЧАЛОПЕРИОДА и КОНЕЦПЕРИОДА. Без него платформа молча возьмёт ДЕНЬ.
    pub period: String,
    /// Готовое выражение вместо пары func+field.
    pub expr: String,
    /// Псевдоним колонки результата.
    pub as_: String,
}

/// Русские имена ключей колонки, принимаемые наравне с латинскими.
const FIELD_ALIASES: &[(&str, &str)] = &[
    ("поле", "field"),
    ("функция", "func"),
    ("период", "period"),
    ("выражение", "expr"),
    ("как", "as"),
];

/// Русские имена ключей входа, принимаемые наравне с латинскими.
///
/// «база» в алиасы НЕ входит: `base` обязателен в схеме и должен иметь одно написание —
/// иначе required требует латинский ключ, а схема разрешает и русский, и вызов с «база»
/// отвергается валидатором раньше, чем алиас успевает сработать.
const INPUT_ALIASES: &[(&str, &str)] = &[
    ("источник", "from"),
    ("псевдоним", "alias"),
    ("параметрыТаблицы", "table_params"),
    ("поля", "select"),
    ("отбор", "where"),
    ("группировка", "group_by"),
    ("порядок", "order_by"),
    ("различные", "distinct"),
    ("первые", "top"),
    ("соединения", "joins"),
    ("итоги", "totals"),
    ("параметры", "params"),
];

/// Переводит русские ключи в латинские. Латинский ключ, если он уже задан, имеет
/// приоритет: пришли обе формы — побеждает каноническая.
fn rename_keys(raw: &mut Map<String, Value>, aliases: &[(&str, &str)]) {
    for (ru, en) in aliases {
        let Some(v) = raw.remove(*ru) else { continue };
        if !raw.contains_key(*en) {
            raw.insert((*en).to_string(), v);
        }
    }
}

/// Если значение — JSON-СТРОКА, внутри которой лежит валидный JSON-массив, возвращает
/// сам массив; иначе значение как есть.
///
/// Строка с одним условием («Дата МЕЖДУ &Н И &К») массивом не разбирается и проходит
/// нетронутой — её разворачивает следующий шаг по своим правилам.
fn unwrap_json_in_string(value: Value) -> Value {
    let Value::String(s) = &value else {
        return value;
    };
    let trimmed = s.trim();
    if !trimmed.starts_with('[') {
        return value;
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(v @ Value::Array(_)) => v,
        _ => value,
    }
}

/// Достаёт строковое значение ключа. Не-строка приводится к тексту: модель, приславшая
/// число там, где ждали строку, должна получить ответ базы, а не придирку к типу.
fn text_of(raw: &Map<String, Value>, key: &str) -> String {
    match raw.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

impl<'de> Deserialize<'de> for QueryBuildField {
    /// Принимает колонку и строкой, и объектом.
    ///
    /// Строка — самый частый вид («Ссылка», «Номенклатура.Наименование»), и модели пишут
    /// её строкой, что бы ни говорило описание: прогон 27.08.2026 в Kilo дал отказ
    /// валидации «want object» на каждой попытке.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(d)?;
        if let Value::String(name) = value {
            return Ok(QueryBuildField {
                field: name,
                ..Default::default()
            });
        }
        // Число или булево колонкой — не послабление, а мусор: лояльность распространяется
        // на ФОРМУ (строка вместо объекта), а не на смысл. Go-версия отвергает такое
        // разбором даже там, где схема пропустила (русские ключи идут через
        // additionalProperties), и здесь это поведение сохранено.
        let Value::Object(mut raw) = value else {
            return Err(serde::de::Error::custom(
                "колонка: ожидается строка или объект",
            ));
        };
        rename_keys(&mut raw, FIELD_ALIASES);
        Ok(QueryBuildField {
            field: text_of(&raw, "field"),
            func: text_of(&raw, "func"),
            period: text_of(&raw, "period"),
            expr: text_of(&raw, "expr"),
            as_: text_of(&raw, "as"),
        })
    }
}

/// Присоединяемая таблица.
///
/// Соединения были главной причиной, по которой модель бросала построитель и уходила
/// писать текст руками, — а руками у неё выходит вдвое хуже (41% против 73% удачных
/// выполнений, замер 31.08.2026).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct QueryBuildJoin {
    /// Присоединяемая таблица.
    #[serde(alias = "источник")]
    pub from: String,
    /// Псевдоним. По умолчанию — последняя часть имени.
    #[serde(alias = "псевдоним")]
    pub alias: String,
    /// Вид соединения: left (умолчание), inner, right, full.
    #[serde(alias = "тип")]
    pub r#type: String,
    /// Условие соединения целиком, с псевдонимами обеих таблиц.
    #[serde(alias = "условие")]
    pub on: String,
    /// Параметры виртуальной таблицы присоединяемого источника, по позициям.
    #[serde(alias = "параметрыТаблицы")]
    pub table_params: Vec<String>,
}

/// Строка ИТОГИ.
///
/// Отдельной структурой, а не строкой текста: агрегат и разрез — разные вещи, и склеенные
/// в строку они возвращают ту же болезнь, от которой построитель заведён.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct QueryBuildTotal {
    /// Итоговое выражение: СУММА(Сумма), КОЛИЧЕСТВО(*), псевдоним колонки.
    #[serde(alias = "выражение")]
    pub expr: String,
    /// Псевдоним итоговой колонки.
    #[serde(alias = "как", rename = "as")]
    pub as_: String,
    /// Разрез итога: имя поля или ОБЩИЕ. Пусто — общий итог.
    #[serde(alias = "по")]
    pub by: String,
}

/// Параметр запроса со ЗНАЧЕНИЕМ, а не только именем.
///
/// Нужен там, где значение нельзя написать текстом: счёт плана счетов, ссылка на элемент
/// справочника. Замер 31.08.2026: `СчетДт = &Счет` с кодом счёта строкой падает «нельзя
/// сравнивать поля несовместимых типов» — полю нужна ССЫЛКА, а модель об этом не знает
/// и повторяет попытку.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct QueryBuildParam {
    /// Имя параметра без амперсанда.
    #[serde(alias = "имя")]
    pub name: String,
    /// Значение параметра: число, строка, дата.
    #[serde(alias = "значение")]
    pub value: Option<Value>,
    /// КОД счёта: сервер найдёт его в плане счетов и подставит ссылку.
    #[serde(alias = "счет", alias = "счёт")]
    pub account: String,
    /// Ссылка на предопределённый элемент: Справочник.Валюты.Рубль.
    #[serde(alias = "ссылка")]
    pub r#ref: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueryBuildInput {
    pub base: String,
    pub from: String,
    pub alias: String,
    pub table_params: Vec<String>,
    pub select: Vec<QueryBuildField>,
    pub r#where: Vec<String>,
    pub group_by: Vec<String>,
    pub order_by: Vec<String>,
    pub distinct: bool,
    pub top: i64,
    pub joins: Vec<QueryBuildJoin>,
    pub totals: Vec<QueryBuildTotal>,
    pub params: Vec<QueryBuildParam>,
}

impl<'de> Deserialize<'de> for QueryBuildInput {
    /// Принимает вход и с латинскими ключами, и с русскими.
    ///
    /// Русские остаются рабочими намеренно: они разосланы в примерах и методиках,
    /// а модель, однажды написавшая «источник», должна получить запрос, а не отказ
    /// валидации.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;

        let Value::Object(mut raw) = Value::deserialize(d)? else {
            return Err(D::Error::custom("query_build: ожидается объект"));
        };
        rename_keys(&mut raw, INPUT_ALIASES);

        // Массив, завёрнутый в СТРОКУ («select»: "[\"Ссылка\"]") — типовой флейк слабых
        // моделей: 17 отказов подряд в живой сессии 02.09.2026, модель так и не подобрала
        // форму. Строка, внутри которой валидный JSON-массив, разворачивается в массив.
        for key in [
            "select",
            "where",
            "group_by",
            "order_by",
            "table_params",
            "params",
            "joins",
            "totals",
        ] {
            if let Some(v) = raw.remove(key) {
                raw.insert(key.to_string(), unwrap_json_in_string(v));
            }
        }
        // Одно условие строкой — то же, что список из одного элемента. Схема это
        // разрешает; здесь строка разворачивается в список, чтобы дальше по коду был
        // один вид данных, а не два.
        for key in ["where", "group_by", "order_by", "table_params"] {
            if let Some(Value::String(s)) = raw.get(key) {
                let wrapped = Value::Array(vec![Value::String(s.clone())]);
                raw.insert(key.to_string(), wrapped);
            }
        }

        let list = |raw: &Map<String, Value>, key: &str| -> Vec<String> {
            match raw.get(key) {
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|i| match i {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect(),
                _ => Vec::new(),
            }
        };
        // Обобщённая, а не замыкание: списки разного типа (колонки, соединения, итоги,
        // параметры) разбираются одним правилом — «есть массив, разбираем; нет, пусто».
        fn typed<T: serde::de::DeserializeOwned, E: serde::de::Error>(
            raw: &Map<String, Value>,
            key: &str,
        ) -> Result<Vec<T>, E> {
            match raw.get(key) {
                Some(v @ Value::Array(_)) => serde_json::from_value(v.clone()).map_err(E::custom),
                _ => Ok(Vec::new()),
            }
        }

        Ok(QueryBuildInput {
            base: text_of(&raw, "base"),
            from: text_of(&raw, "from"),
            alias: text_of(&raw, "alias"),
            table_params: list(&raw, "table_params"),
            select: typed::<_, D::Error>(&raw, "select")?,
            r#where: list(&raw, "where"),
            group_by: list(&raw, "group_by"),
            order_by: list(&raw, "order_by"),
            distinct: matches!(raw.get("distinct"), Some(Value::Bool(true))),
            top: raw.get("top").and_then(Value::as_i64).unwrap_or(0),
            joins: typed::<_, D::Error>(&raw, "joins")?,
            totals: typed::<_, D::Error>(&raw, "totals")?,
            params: typed::<_, D::Error>(&raw, "params")?,
        })
    }
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct BuildReply {
    #[serde(rename = "запрос")]
    query: String,
    #[serde(rename = "параметры")]
    params: Vec<String>,
    /// Значения, вычисленные при сборке (ссылка счёта по коду, предопределённое значение).
    /// Сборка и выполнение — разные вызовы, и всё вычисленное живёт между ними только
    /// здесь. Не донести их до `query` — значит дать выполнить запрос со строкой вместо
    /// ссылки: сравнение не упадёт, а вернёт ноль строк, неотличимый от «в базе пусто».
    /// BTreeMap, а не `Map` serde_json: ключи печатаются модели и обязаны идти в
    /// одном порядке от вызова к вызову. `serde_json` собран с `preserve_order`
    /// (порядок полей базы нужен разбору запроса), поэтому стабильность здесь даёт
    /// только явная сортировка — Go сортирует ключи map при сериализации сам.
    #[serde(rename = "значенияПараметров")]
    param_values: BTreeMap<String, Value>,
    #[serde(rename = "представленияПараметров")]
    param_titles: BTreeMap<String, Value>,
}

// ── Отбор, спорящий с параметрами виртуальной таблицы ─────────────────────────
//
// Виртуальная таблица регистра отбирает данные ДВАЖДЫ и по разным правилам: условие в
// скобках (`Счет В ИЕРАРХИИ(&Счет)`) собирает итоги по ветке, а `ГДЕ` фильтрует уже
// собранное. Поставить оба по одному полю — значит попросить платформу сложить субсчета
// и тут же выбросить всё, что не равно родителю. Запрос при этом СИНТАКСИЧЕСКИ ВЕРЕН:
// платформа его принимает, выполняет и возвращает ноль строк.
//
// Ноль строк — худший из возможных исходов, потому что он неотличим от честного «в базе
// пусто». Наблюдалось живьём 31.08.2026 на bu3: `accounts` по счёту 51 давал 8 строк с
// оборотами, а собранный здесь запрос по тому же счёту — 0, четырьмя разными способами
// подряд. Модель верно назвала причину вслух и всё равно не выбралась: инструмент молчал,
// а платформа не возражала.
//
// Поэтому проверка стоит ДО отправки в базу. Она не про синтаксис — синтаксис там
// безупречен — она про смысл, который платформа проверять не обязана.

/// Достаёт имя поля из левой части условия: `Счет В ИЕРАРХИИ(&Счет)` → `Счет`,
/// `ОстаткиИОбороты.Счет = &Счет` → `Счет`.
///
/// Пустая строка — условие не распознано, и тогда проверка молчит: догадка о смысле хуже
/// отсутствия проверки.
fn condition_field(condition: &str) -> String {
    let trimmed = condition.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Отрезаем по первому оператору сравнения или ключевому слову отбора. Искать надо
    // без учёта регистра, а резать — по исходной строке, поэтому позиция считается
    // в верхнерегистровой копии и переносится в исходную по числу символов.
    let upper: Vec<char> = trimmed.to_uppercase().chars().collect();
    let source: Vec<char> = trimmed.chars().collect();
    let mut cut = source.len();
    for op in [
        " В ИЕРАРХИИ",
        " В(",
        " В ",
        "=",
        ">=",
        "<=",
        "<>",
        ">",
        "<",
        " МЕЖДУ ",
        " ПОДОБНО ",
        " ЕСТЬ ",
    ] {
        let pattern: Vec<char> = op.chars().collect();
        if let Some(i) = upper.windows(pattern.len()).position(|w| w == pattern) {
            if i > 0 {
                cut = i;
                break;
            }
        }
    }
    // Регистр меняет длину в символах у отдельных букв крайне редко, но верхний регистр
    // строится посимвольно из той же строки, поэтому индексы совпадают по числу символов.
    let mut s: String = source[..cut.min(source.len())].iter().collect();
    s = s.trim().to_string();
    // Псевдоним таблицы отбрасываем: спорят поля, а не способ их назвать.
    if let Some(i) = s.rfind('.') {
        s = s[i + '.'.len_utf8()..].to_string();
    }
    // Осталось что-то, кроме имени поля, — не наш случай.
    if s.is_empty() || s.contains(['(', ')', ' ', ',', '+', '-', '*', '/', '&']) {
        return String::new();
    }
    s
}

/// Возвращает отказ, если одно и то же поле отбирается и параметром таблицы, и условием
/// ГДЕ. Совпадение по имени поля — достаточный признак: разные поля с одним именем
/// в одной таблице невозможны.
fn check_filter_conflict(
    base: &str,
    table_params: &[String],
    conditions: &[String],
) -> Result<(), Refusal> {
    if table_params.is_empty() || conditions.is_empty() {
        return Ok(());
    }
    let mut in_params: BTreeMap<String, String> = BTreeMap::new();
    for p in table_params {
        let field = condition_field(p);
        if !field.is_empty() {
            in_params.insert(field.to_uppercase(), p.trim().to_string());
        }
    }
    for c in conditions {
        let field = condition_field(c);
        if field.is_empty() {
            continue;
        }
        let Some(param) = in_params.get(&field.to_uppercase()) else {
            continue;
        };
        return Err(Refusal::new(
            Kind::BadRequest,
            format!("отбор по полю {field} спорит с параметром таблицы"),
            format!(
                "параметр таблицы уже отбирает по этому полю ({param}), а условие ГДЕ ({}) \
                 фильтрует то, что параметр собрал; вместе они дают ноль строк, и платформа \
                 на это не пожалуется — запрос синтаксически верен",
                c.trim()
            ),
        )
        .hint(format!(
            "уберите условие {:?} из where — параметр таблицы уже делает нужный отбор",
            c.trim()
        ))
        .hint(
            "если нужен ровно этот счёт без субсчетов — уберите В ИЕРАРХИИ из параметра \
             таблицы, а не добавляйте ГДЕ",
        )
        .hint("итоги по счёту с субсчетами считает инструмент accounts — без языка запросов")
        .stamp(base));
    }
    Ok(())
}

/// Поле запроса в том виде, который ждёт расширение: строка для простого имени, объект
/// для функции или псевдонима. Строкой короче, но схема одна.
fn field_payload(f: &QueryBuildField) -> Value {
    if f.func.is_empty() && f.expr.is_empty() && f.as_.is_empty() && f.period.is_empty() {
        return Value::String(f.field.clone());
    }
    let mut obj = Map::new();
    let mut put = |key: &str, v: &str| {
        if !v.is_empty() {
            obj.insert(key.to_string(), Value::String(v.to_string()));
        }
    };
    put("поле", &f.field);
    put("функция", &f.func);
    put("период", &f.period);
    put("выражение", &f.expr);
    put("как", &f.as_);
    Value::Object(obj)
}

impl Set {
    /// Разбирает запрос в структуру: таблицы, поля, соединения, отборы, параметры.
    pub fn query_parse(&self, input: &QueryParseInput) -> Result<String, Refusal> {
        if input.query.trim().is_empty() {
            return Err(Refusal::new(
                Kind::BadRequest,
                "текст запроса пуст",
                "поле query обязательно",
            ));
        }
        let client = self.channel_for(&input.base)?;
        let reply: ParseReply = client
            .tell("parse", &json!({ "query": input.query }))
            // Сюда приходят с чужим текстом: подсказка о виртуальных таблицах и языке
            // нужна не меньше, чем при проверке своего.
            .map_err(|e| enrich_query_refusal(e, &input.query, &BTreeMap::new()))?;

        let mut out = format!(
            "Запрос разобран: запросов в пакете {}.\n",
            reply.queries_in_batch
        );
        if !reply.tables.is_empty() {
            out.push_str(&format!(
                "Таблицы базы ({}): {}\n",
                reply.tables.len(),
                reply.tables.join(", ")
            ));
        }
        if !reply.temp_tables.is_empty() {
            out.push_str(&format!(
                "Временные таблицы: {}\n",
                reply.temp_tables.join(", ")
            ));
        }
        if !reply.params.is_empty() {
            out.push_str(&format!("Параметры: {}\n", reply.params.join(", ")));
        }
        out.push_str("\nУстройство по запросам пакета:\n");
        out.push_str(&indent_json(&reply.queries));
        Ok(out)
    }

    /// Собирает текст запроса из структуры силами платформы.
    pub fn query_build(&self, input: &QueryBuildInput) -> Result<String, Refusal> {
        // Построитель позван — проверка текста открыта снова, каким бы ни был исход.
        self.gate.on_build_called();
        if input.from.trim().is_empty() {
            return Err(Refusal::new(
                Kind::BadRequest,
                "источник не назван",
                "ключ from (он же «источник») обязателен: имя таблицы вида Справочник.Номенклатура",
            ));
        }
        if input.select.is_empty() {
            return Err(Refusal::new(
                Kind::BadRequest,
                "поля не заданы",
                "ключ select (он же «поля») обязателен: хотя бы одна колонка результата",
            ));
        }
        let client = self.channel_for(&input.base)?;

        let mut payload = Map::new();
        payload.insert("источник".into(), Value::String(input.from.clone()));
        payload.insert(
            "поля".into(),
            Value::Array(input.select.iter().map(field_payload).collect()),
        );
        if !input.alias.is_empty() {
            payload.insert("псевдоним".into(), Value::String(input.alias.clone()));
        }
        for (key, v) in [
            ("параметрыТаблицы", &input.table_params),
            ("отбор", &input.r#where),
            ("группировка", &input.group_by),
            ("порядок", &input.order_by),
        ] {
            if !v.is_empty() {
                payload.insert(key.to_string(), json!(v));
            }
        }
        if input.distinct {
            payload.insert("различные".into(), Value::Bool(true));
        }
        if input.top > 0 {
            payload.insert("первые".into(), json!(input.top));
        }
        if !input.joins.is_empty() {
            let joins: Vec<Value> = input
                .joins
                .iter()
                .map(|j| {
                    let mut obj = Map::new();
                    obj.insert("источник".into(), Value::String(j.from.clone()));
                    obj.insert("условие".into(), Value::String(j.on.clone()));
                    if !j.alias.is_empty() {
                        obj.insert("псевдоним".into(), Value::String(j.alias.clone()));
                    }
                    if !j.r#type.is_empty() {
                        obj.insert("тип".into(), Value::String(j.r#type.clone()));
                    }
                    if !j.table_params.is_empty() {
                        obj.insert("параметрыТаблицы".into(), json!(j.table_params));
                    }
                    Value::Object(obj)
                })
                .collect();
            payload.insert("соединения".into(), Value::Array(joins));
        }
        if !input.totals.is_empty() {
            let totals: Vec<Value> = input
                .totals
                .iter()
                .map(|t| {
                    let mut obj = Map::new();
                    obj.insert("выражение".into(), Value::String(t.expr.clone()));
                    if !t.as_.is_empty() {
                        obj.insert("как".into(), Value::String(t.as_.clone()));
                    }
                    if !t.by.is_empty() {
                        obj.insert("по".into(), Value::String(t.by.clone()));
                    }
                    Value::Object(obj)
                })
                .collect();
            payload.insert("итоги".into(), Value::Array(totals));
        }
        if !input.params.is_empty() {
            let params: Vec<Value> = input
                .params
                .iter()
                .map(|p| {
                    let mut obj = Map::new();
                    obj.insert("имя".into(), Value::String(p.name.clone()));
                    if let Some(v) = &p.value {
                        obj.insert("значение".into(), v.clone());
                    }
                    if !p.account.is_empty() {
                        obj.insert("счет".into(), Value::String(p.account.clone()));
                    }
                    if !p.r#ref.is_empty() {
                        obj.insert("ссылка".into(), Value::String(p.r#ref.clone()));
                    }
                    Value::Object(obj)
                })
                .collect();
            payload.insert("параметры".into(), Value::Array(params));
        }

        // Спор отбора с параметрами таблицы платформа не ловит: такой запрос верен и пуст.
        check_filter_conflict(&input.base, &input.table_params, &input.r#where)?;

        let reply: BuildReply = client
            .tell("build", &Value::Object(payload))
            .map_err(|e| enrich_query_refusal(e, &input.from, &BTreeMap::new()))?;

        // Собранный платформой текст одобрен к выполнению как есть.
        self.gate.approve(&reply.query);

        let mut out = String::from("Запрос собран и проверен платформой:\n\n");
        out.push_str(&reply.query);
        if !reply.params.is_empty() {
            out.push_str(&format!(
                "\n\nПараметры к заполнению при выполнении: {}",
                reply.params.join(", ")
            ));
        }
        // Вычисленные значения печатаются ГОТОВЫМИ к подстановке. Код счёта уже превращён
        // в ссылку плана счетов, и подставить вместо неё строку «51» — значит получить ноль
        // строк вместо данных, без единого признака ошибки. Поэтому здесь не совет «не
        // забудьте про ссылку», а буквальный кусок JSON, который остаётся скопировать.
        if !reply.param_values.is_empty() {
            let stable: Value = sorted_json(&Value::Object(
                reply
                    .param_values
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ));
            if let Ok(ready) = serde_json::to_string(&stable) {
                out.push_str(
                    "\n\nЗначения, вычисленные при сборке, — подставь их в query ДОСЛОВНО:\n",
                );
                out.push_str(&ready);
                for (name, title) in &reply.param_titles {
                    let title = match title {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    out.push_str(&format!("\n  {name} — {title}"));
                }
                out.push_str(
                    "\nСтрока вместо этого значения (например 51 вместо ссылки счёта) \
                     не вызовет ошибки — запрос вернёт ноль строк.",
                );
            }
        }
        out.push_str("\n\nВыполнить — инструмент query с этим текстом.");
        Ok(out)
    }
}

/// Схема входа `query_build` — для MCP-слоя.
///
/// Собирается вручную, а не выводится из типа: выведенная схема знает только строгую
/// форму, а инструмент обязан принимать и лояльную — иначе SDK отвергает вызов до
/// обработчика, модель получает «want object» и уходит сочинять текст запроса руками,
/// ради чего построитель и заведён.
///
/// Ключи ЛАТИНСКИЕ. Причина внешняя: Anthropic API требует `^[a-zA-Z0-9_.-]{1,64}$` и
/// отвергает инструмент ЦЕЛИКОМ, если хоть один ключ ей не отвечает — объявленные русские
/// алиасы вернули бы ту же болезнь, только раньше: инструмент не доехал бы до модели
/// вовсе. Русские имена в схеме не объявлены, но и не запрещены
/// (`additionalProperties`), и переводятся при разборе.
///
/// `base` остаётся единственным обязательным. Источник и поля из required убраны
/// сознательно: валидатор проверяет `anyOf` раньше `required`, и вызов без `base`
/// отвечал бы нечитаемым «did not validate against any of [...]» вместо «назови base».
/// Их отсутствие ловит обработчик, где отказ пишется человеческим языком.
pub fn query_build_schema() -> Value {
    /// Массив, который принимается и строкой с JSON внутри.
    fn wrapped(items: Value, description: &str) -> Value {
        json!({
            "anyOf": [
                {"type": "string", "description": "JSON-массив, случайно завёрнутый в строку — принимается и разбирается"},
                {"type": "array", "items": items, "description": description}
            ]
        })
    }
    /// Список условий: строка ИЛИ массив строк.
    fn conditions(description: &str) -> Value {
        json!({
            "anyOf": [
                {"type": "string", "description": "Одно условие строкой — то же, что список из одного элемента"},
                {"type": "array", "items": {"type": "string"}}
            ],
            "description": description
        })
    }

    let field_object = json!({
        "type": "object",
        "description": "Колонка с агрегатом, выражением или псевдонимом",
        // Русские ключи колонки в схеме не объявлены — она обязана быть латинской, —
        // но пропускаются к разбору, где алиасы переводятся. Без этого смешанная форма
        // (латинский select с русскими ключами внутри) отвергалась валидатором как
        // «anyOf: did not validate», а именно её модель и пишет чаще всего.
        "additionalProperties": true,
        "properties": {
            "field": {"type": "string", "description": "Имя поля источника, можно через точку: Номенклатура, Ссылка.Дата"},
            "func": {"type": "string", "description": "Функция над полем: СУММА, КОЛИЧЕСТВО, МАКСИМУМ, МИНИМУМ, СРЕДНЕЕ, ГОД, НАЧАЛОПЕРИОДА, КОНЕЦПЕРИОДА"},
            "period": {"type": "string", "description": "Период для НАЧАЛОПЕРИОДА и КОНЕЦПЕРИОДА: МИНУТА, ЧАС, ДЕНЬ, НЕДЕЛЯ, ДЕКАДА, МЕСЯЦ, КВАРТАЛ, ПОЛУГОДИЕ, ГОД. Для них обязателен: без него платформа молча возьмёт ДЕНЬ"},
            "expr": {"type": "string", "description": "Готовое выражение вместо пары func+field: ВЫБОР КОГДА … КОНЕЦ, арифметика. Псевдоним таблицы указывать явно"},
            "as": {"type": "string", "description": "Псевдоним колонки результата"}
        }
    });

    json!({
        "type": "object",
        "required": ["base"],
        "additionalProperties": true,
        "properties": {
            "base": {"type": "string", "description": "Имя базы 1С из реестра. Обязательно; перечень — bases с action=list"},
            "from": {"type": "string", "description": "Таблица-источник: Справочник.Номенклатура, Документ.РеализацияТоваровУслуг.Товары, РегистрНакопления.ТоварыНаСкладах.Остатки"},
            "alias": {"type": "string", "description": "Псевдоним источника в запросе. По умолчанию — последняя часть имени таблицы"},
            "distinct": {"type": "boolean", "description": "ВЫБРАТЬ РАЗЛИЧНЫЕ"},
            "top": {"type": "integer", "description": "ВЫБРАТЬ ПЕРВЫЕ N"},
            "select": wrapped(
                json!({"anyOf": [
                    {"type": "string", "description": "Имя поля источника, можно через точку: Ссылка, Номенклатура.Наименование"},
                    field_object
                ]}),
                "Колонки результата. Каждая — либо строка с именем поля («Ссылка», «Номенклатура.Наименование»), либо объект {field, func, expr, as} для агрегата или псевдонима"
            ),
            "where": conditions("Условия ГДЕ, по одному на элемент: Ссылка.Проведен, Дата МЕЖДУ &Начало И &Конец. Соединяются через И"),
            "group_by": conditions("Поля СГРУППИРОВАТЬ ПО"),
            "order_by": conditions("Поля УПОРЯДОЧИТЬ ПО — псевдонимы колонок результата, не выражения источника"),
            "table_params": conditions("Параметры виртуальной таблицы по позициям, как в скобках: для Остатки — [\"&НаДату\", \"Склад = &Склад\"]. Пустая строка пропускает позицию"),
            "joins": wrapped(json!({
                "type": "object",
                "additionalProperties": true,
                "properties": {
                    "from": {"type": "string", "description": "Присоединяемая таблица: Справочник.Контрагенты, РегистрСведений.ЦеныНоменклатуры.СрезПоследних"},
                    "alias": {"type": "string", "description": "Псевдоним присоединяемой таблицы. По умолчанию — последняя часть имени"},
                    "type": {"type": "string", "description": "Вид соединения: left (умолчание), inner, right, full"},
                    "on": {"type": "string", "description": "Условие соединения целиком, с псевдонимами обеих таблиц: Док.Контрагент = Контрагенты.Ссылка"},
                    "table_params": {"type": "array", "items": {"type": "string"}, "description": "Параметры виртуальной таблицы присоединяемого источника, по позициям"}
                }
            }), "Присоединяемые таблицы. Каждая — {from, on, type, alias}. Поля присоединённых таблиц называйте через их псевдоним"),
            "totals": wrapped(json!({
                "type": "object",
                "additionalProperties": true,
                "properties": {
                    "expr": {"type": "string", "description": "Итоговое выражение: СУММА(Сумма), КОЛИЧЕСТВО(*), или псевдоним колонки результата"},
                    "as": {"type": "string", "description": "Псевдоним итоговой колонки"},
                    "by": {"type": "string", "description": "Разрез итога: имя поля или ОБЩИЕ. Пусто — общий итог"}
                }
            }), "Строки ИТОГИ: {expr, by}. Разрез by — имя поля или ОБЩИЕ"),
            "params": wrapped(json!({
                "type": "object",
                "additionalProperties": true,
                "properties": {
                    "name": {"type": "string", "description": "Имя параметра без амперсанда: Счет, Начало, Контрагент"},
                    "value": {"description": "Значение параметра: число, строка, дата в формате 2026-01-01"},
                    "account": {"type": "string", "description": "КОД счёта: сервер найдёт его в плане счетов регистра и подставит ссылку. Для отбора по счёту пишите условие Счет В ИЕРАРХИИ(&Имя) — сравнение СчетДт = &Имя со строкой не работает"},
                    "ref": {"type": "string", "description": "Ссылка на предопределённый элемент: Справочник.Валюты.Рубль, Перечисление.СтавкиНДС.НДС20"}
                }
            }), "Значения параметров запроса. Нужны там, где значение не пишется текстом: счёт (account), ссылка (ref)")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn разобрать(json: &str) -> QueryBuildInput {
        serde_json::from_str(json).expect("разбор ввода не удался")
    }

    fn set() -> Set {
        let dir = std::env::temp_dir().join(format!("gfdata-qs-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("bases.json");
        let _ = std::fs::remove_file(&path);
        let mut s = Set::new("0.1.0");
        s.registry_path = Some(path);
        s
    }

    // Колонки «поля» приходят от модели и строкой, и объектом. Обе формы обязаны
    // раскладываться в одну структуру — иначе отказ валидации выглядит для модели как
    // поломка инструмента, и она уходит писать текст руками.

    #[test]
    fn поля_строкой_и_объектом() {
        let in_ = разобрать(
            r#"{"base":"ut11","источник":"Справочник.Номенклатура",
                "поля":["Ссылка", {"поле":"Наименование","как":"Имя"},
                        {"функция":"КОЛИЧЕСТВО","поле":"Ссылка","как":"Всего"}]}"#,
        );
        assert_eq!(in_.select.len(), 3, "колонок должно быть 3");
        assert_eq!(in_.select[0].field, "Ссылка");
        assert_eq!(in_.select[0].as_, "", "строка не должна давать псевдоним");
        assert_eq!(in_.select[1].field, "Наименование");
        assert_eq!(in_.select[1].as_, "Имя");
        assert_eq!(in_.select[2].func, "КОЛИЧЕСТВО", "агрегат потерян");
        // Русские ключи входа переведены заодно.
        assert_eq!(in_.from, "Справочник.Номенклатура");
    }

    #[test]
    fn число_колонкой_отвергается_разбором() {
        // Русские ключи схемой не объявлены и проходят через additionalProperties,
        // значит форму элемента проверяет разбор — и обязан проверять строго, иначе
        // «совместимость» означала бы «что угодно вместо колонки».
        let err = serde_json::from_str::<QueryBuildInput>(
            r#"{"base":"ut11","источник":"Справочник.Номенклатура","поля":[42]}"#,
        );
        assert!(err.is_err(), "число в «поля» должно отвергаться разбором");
    }

    #[test]
    fn латинские_ключи_побеждают_русские() {
        let in_ = разобрать(
            r#"{"base":"ut11","from":"Документ.Правильный","источник":"Документ.Старый",
                "select":["Ссылка"]}"#,
        );
        assert_eq!(
            in_.from, "Документ.Правильный",
            "пришли обе формы — побеждает каноническая"
        );
    }

    // Стрингифицированный список — 17 отказов подряд в живой сессии 02.09.2026.

    #[test]
    fn массив_завёрнутый_в_строку_разворачивается() {
        let in_ = разобрать(
            r#"{"base":"bu3","from":"Документ.Тест",
                "select":"[\"Ссылка\", {\"field\": \"СуммаДокумента\", \"func\": \"СУММА\", \"as\": \"Сумма\"}]",
                "where":"[\"Дата МЕЖДУ &Н И &К\", \"Проведен\"]"}"#,
        );
        assert_eq!(in_.select.len(), 2, "select: ждали 2 поля");
        assert_eq!(in_.select[1].func, "СУММА");
        assert_eq!(in_.r#where.len(), 2, "where: ждали 2 условия");
        assert_eq!(in_.r#where[0], "Дата МЕЖДУ &Н И &К");
    }

    #[test]
    fn одно_условие_строкой_это_список_из_одного() {
        let in_ = разобрать(
            r#"{"base":"bu3","from":"Документ.СписаниеСРасчетногоСчета","select":["Ссылка"],
                "where":"Дата МЕЖДУ &Н И &К","group_by":"Ссылка","order_by":"Ссылка",
                "table_params":"&Н"}"#,
        );
        for (имя, got) in [
            ("where", &in_.r#where),
            ("group_by", &in_.group_by),
            ("order_by", &in_.order_by),
            ("table_params", &in_.table_params),
        ] {
            assert_eq!(got.len(), 1, "{имя}: ждали 1 элемент, получено {got:?}");
        }
        assert_eq!(in_.r#where[0], "Дата МЕЖДУ &Н И &К", "условие исказилось");
    }

    #[test]
    fn похожая_на_массив_но_битая_строка_остаётся_условием() {
        // Отказ здесь был бы придиркой к форме: пусть база скажет, что не так со смыслом.
        let in_ = разобрать(
            r#"{"base":"bu3","from":"Документ.Тест","select":["Ссылка"],"where":"[не json"}"#,
        );
        assert_eq!(in_.r#where, vec!["[не json"]);
    }

    #[test]
    fn соединения_итоги_и_параметры_разбираются() {
        let in_ = разобрать(
            r#"{"base":"bu3","from":"Документ.РеализацияТоваровУслуг","alias":"Док",
                "select":["Ссылка"],
                "joins":[{"from":"Справочник.Контрагенты","alias":"К","type":"inner",
                          "on":"Док.Контрагент = К.Ссылка"}],
                "totals":[{"expr":"СУММА(СуммаДокумента)","by":"ОБЩИЕ"}],
                "params":[{"name":"Счет","account":"51"}],
                "distinct":true,"top":10}"#,
        );
        assert_eq!(in_.joins.len(), 1);
        assert_eq!(in_.joins[0].r#type, "inner");
        assert_eq!(in_.joins[0].on, "Док.Контрагент = К.Ссылка");
        assert_eq!(in_.totals[0].by, "ОБЩИЕ");
        assert_eq!(in_.params[0].account, "51");
        assert!(in_.distinct);
        assert_eq!(in_.top, 10);
        assert_eq!(in_.alias, "Док");
    }

    #[test]
    fn русские_ключи_соединений_и_итогов_принимаются() {
        let in_ = разобрать(
            r#"{"base":"bu3","источник":"Документ.Тест","поля":["Ссылка"],
                "соединения":[{"источник":"Справочник.Контрагенты","условие":"А = Б","тип":"left"}],
                "итоги":[{"выражение":"СУММА(Сумма)","по":"Контрагент"}],
                "параметры":[{"имя":"Счет","счет":"51"}]}"#,
        );
        assert_eq!(in_.joins[0].from, "Справочник.Контрагенты");
        assert_eq!(in_.joins[0].on, "А = Б");
        assert_eq!(in_.totals[0].expr, "СУММА(Сумма)");
        assert_eq!(in_.totals[0].by, "Контрагент");
        assert_eq!(in_.params[0].account, "51");
    }

    // Спор отбора с параметром виртуальной таблицы — случай, который платформа
    // пропускает молча. Запрос верен синтаксически и возвращает ноль строк; ноль строк
    // неотличим от «в базе пусто».

    #[test]
    fn поле_условия_достаётся_из_левой_части() {
        for (вход, ждём) in [
            ("Счет В ИЕРАРХИИ(&Счет)", "Счет"),
            ("ОстаткиИОбороты.Счет = &Счет", "Счет"),
            ("Обороты.Счет=&Счет", "Счет"),
            ("Дата МЕЖДУ &Н И &К", "Дата"),
            ("Склад В (&Склады)", "Склад"),
            ("Номенклатура.Наименование = &Имя", "Наименование"),
            ("", ""),
            // Не имя поля — проверка обязана молчать: догадка о смысле хуже её отсутствия.
            ("СУММА(Оборот) > 100", ""),
        ] {
            assert_eq!(condition_field(вход), ждём, "condition_field({вход:?})");
        }
    }

    #[test]
    fn спор_отбора_ловится_до_базы() {
        // Ровно тот вызов, что дал 0 строк на bu3 31.08.2026.
        let err = check_filter_conflict(
            "bu3",
            &[
                "&Н".into(),
                "&К".into(),
                "Месяц".into(),
                "Счет В ИЕРАРХИИ(&Счет)".into(),
            ],
            &["Счет = &Счет".into()],
        )
        .expect_err("спор не пойман — запрос уйдёт в базу и вернёт ноль строк");
        let текст = err.to_string();
        for кусок in ["ОТКАЗ", "bu3", "Счет", "ноль строк", "accounts"] {
            assert!(текст.contains(кусок), "в отказе нет {кусок:?}:\n{текст}");
        }
    }

    #[test]
    fn спор_отбора_не_мешает_законному_запросу() {
        let параметры = [
            "&Н".to_string(),
            "&К".to_string(),
            "Месяц".to_string(),
            "Счет В ИЕРАРХИИ(&Счет)".to_string(),
        ];
        // Разные поля в параметрах и в отборе — законно и обязано проходить.
        assert!(check_filter_conflict("bu3", &параметры, &["Организация = &Орг".into()]).is_ok());
        // Отбор без параметров таблицы.
        assert!(check_filter_conflict("bu3", &[], &["Счет = &Счет".into()]).is_ok());
        // Параметры без отбора.
        assert!(check_filter_conflict("bu3", &параметры, &[]).is_ok());
    }

    // Обработчик: отказы до канала. Их видит модель, и они обязаны называть ключ.

    #[test]
    fn источник_обязателен() {
        let err = set()
            .query_build(&QueryBuildInput {
                base: "bu3".into(),
                select: vec![QueryBuildField {
                    field: "Ссылка".into(),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(err.kind, Kind::BadRequest);
        assert!(err.to_string().contains("источник не назван"), "{err}");
    }

    #[test]
    fn поля_обязательны() {
        let err = set()
            .query_build(&QueryBuildInput {
                base: "bu3".into(),
                from: "Справочник.Валюты".into(),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(err.kind, Kind::BadRequest);
        assert!(err.to_string().contains("поля не заданы"), "{err}");
    }

    #[test]
    fn построитель_открывает_гейт_даже_при_отказе() {
        // Гейт открывается ДО проверок входа: построитель позван — проверка текста
        // снова разрешена, каким бы ни был исход. Иначе неудачный вызов построителя
        // запирал бы модель в тупике.
        let s = set();
        s.gate.on_check_refused("ВЫБРАТЬ 1");
        assert!(!s.gate.check_allowed().0, "гейт должен быть закрыт");
        let _ = s.query_build(&QueryBuildInput {
            base: "bu3".into(),
            ..Default::default()
        });
        assert!(
            s.gate.check_allowed().0,
            "после вызова построителя проверка обязана открыться"
        );
    }

    #[test]
    fn текст_запроса_обязателен_для_разбора() {
        let err = set()
            .query_parse(&QueryParseInput {
                base: "bu3".into(),
                query: "   ".into(),
            })
            .unwrap_err();
        assert_eq!(err.kind, Kind::BadRequest);
        assert!(err.to_string().contains("текст запроса пуст"), "{err}");
    }

    // Схема — это то, что модель ВИДИТ до вызова. Она обязана пропускать всё, что
    // понимает разбор: иначе валидатор отвергает вызов раньше нашего кода и лояльность
    // мертва.

    #[test]
    fn схема_принимает_строку_во_всех_списочных_полях() {
        let s = query_build_schema();
        for ключ in [
            "select",
            "where",
            "group_by",
            "order_by",
            "table_params",
            "params",
            "joins",
            "totals",
        ] {
            let свойство = &s["properties"][ключ];
            let варианты = свойство["anyOf"]
                .as_array()
                .unwrap_or_else(|| panic!("{ключ}: нет anyOf"));
            assert!(
                варианты.iter().any(|v| v["type"] == "string"),
                "{ключ}: схема не принимает строку"
            );
        }
    }

    #[test]
    fn схема_допускает_колонку_строкой_и_объектом() {
        let s = query_build_schema();
        let select = &s["properties"]["select"]["anyOf"];
        assert_eq!(select[0]["type"], "string");
        assert_eq!(select[1]["type"], "array");
        let элемент = &select[1]["items"]["anyOf"];
        assert_eq!(элемент[0]["type"], "string");
        assert_eq!(элемент[1]["type"], "object");
    }

    #[test]
    fn база_единственное_обязательное_поле() {
        // Источник и поля из required убраны сознательно: валидатор проверяет anyOf
        // раньше required, и вызов без base отвечал бы нечитаемым «did not validate
        // against any of [...]» вместо «назови base».
        let s = query_build_schema();
        assert_eq!(s["required"].as_array().unwrap().len(), 1);
        assert_eq!(s["required"][0], "base");
    }

    #[test]
    fn схема_пропускает_русские_ключи_к_разбору() {
        // additionalProperties=true — русские имена не объявлены (схема обязана быть
        // латинской: Anthropic API отвергает инструмент целиком за нелатинский ключ),
        // но и не запрещены: их переводит разбор.
        let s = query_build_schema();
        assert_eq!(s["additionalProperties"], serde_json::json!(true));
        assert_eq!(
            s["properties"]["select"]["anyOf"][1]["items"]["anyOf"][1]["additionalProperties"],
            serde_json::json!(true),
            "смешанная форма (латинский select с русскими ключами внутри) обязана доходить до разбора"
        );
    }
}
