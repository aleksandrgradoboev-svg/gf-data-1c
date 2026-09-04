//! Срез регистра сведений и бухгалтерские итоги по счёту.
//!
//! Оба инструмента собирают текст запроса на стороне сервера. Смысл тот же, что у
//! `register`: имя виртуальной таблицы, порядок границ периода и подстановка измерений
//! не сочиняются заново — их нельзя ошибиться.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::refusal::{Kind, Refusal};

use super::data::QueryReply;
use super::lenient::StringList;
use super::Set;

// ── Срез регистра сведений ───────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct SliceInput {
    pub base: String,
    /// Имя регистра сведений без префикса, например ЦеныНоменклатуры.
    pub register: String,
    /// `СрезПоследних` (умолчание) или `СрезПервых`.
    pub kind: String,
    /// Дата среза. Пусто — на текущий момент.
    pub period: String,
    /// Отбор по полям среза, без слова ГДЕ.
    pub where_: String,
    pub parameters: BTreeMap<String, Value>,
    pub limit: i64,
}

pub const SLICE_NAME: &str = "slice";
pub const SLICE_DESCRIPTION: &str = "Получить срез регистра сведений — значения, действующие \
на дату: цены номенклатуры, курсы валют, ставки, настройки. Запрос собирается сервером: \
измерения и ресурсы регистра подставляются сами, дата уходит параметром среза. Используй это \
вместо запроса к самому регистру: выборка записей вернёт всю историю, и первая попавшаяся \
строка легко сойдёт за действующее значение. Непериодический регистр даёт отказ — у него \
среза не бывает.";

// ── Итоги по счетам ──────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AccountsInput {
    pub base: String,
    /// Код счёта, например 41 или 62.01.
    pub account: String,
    /// `Остатки` (умолчание), `Обороты` или `ОстаткиИОбороты`.
    pub kind: String,
    pub period: String,
    pub start: String,
    pub end: String,
    /// Разбивка периода: Месяц, Квартал, Год, День, Регистратор.
    pub periodicity: String,
    /// Имя регистра бухгалтерии (по умолчанию Хозрасчетный).
    pub register: String,
    /// Показатели: СуммаОстатокДт, СуммаОборотДт и т.п.
    pub resources: StringList,
    pub parameters: BTreeMap<String, Value>,
    pub limit: i64,
}

pub const ACCOUNTS_NAME: &str = "accounts";
pub const ACCOUNTS_DESCRIPTION: &str = "Получить бухгалтерские итоги по счёту: остатки на дату \
(kind=Остатки), обороты за период (kind=Обороты) или полную картину с начальным и конечным \
остатком (kind=ОстаткиИОбороты). Счёт задаётся кодом — 41, 62.01, 51. Разбивку по периодам \
даёт periodicity (Месяц, Квартал, Год) — так собирается движение счёта помесячно, без единой \
строки на языке запросов. Суммы приходят раздельно по дебету и кредиту. Это для конфигураций \
с бухгалтерским учётом; складские и товарные итоги живут в регистрах накопления, их берёт \
инструмент register.";

impl Set {
    /// Срез регистра сведений на дату.
    pub fn slice(&self, input: &SliceInput) -> Result<String, Refusal> {
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
            ("where", &input.where_),
        ] {
            if !value.trim().is_empty() {
                payload.insert(key.into(), json!(value));
            }
        }
        if !input.parameters.is_empty() {
            payload.insert("parameters".into(), json!(input.parameters));
        }
        if input.limit > 0 {
            payload.insert("limit".into(), json!(input.limit));
        }

        let reply: QueryReply = client.tell("slice", &Value::Object(payload))?;
        Ok(super::data::render_table(&client.base().name, &reply))
    }

    /// Бухгалтерские итоги по счёту.
    pub fn accounts(&self, input: &AccountsInput) -> Result<String, Refusal> {
        if input.account.trim().is_empty() {
            return Err(Refusal::new(
                Kind::BadRequest,
                "счёт не назван",
                "поле account обязательно: код счёта, например 41 или 62.01",
            )
            .hint(
                "перечень счетов — metadata с filter=ПланыСчетов, затем object по нужному плану",
            ));
        }
        let client = self.channel_for(&input.base)?;

        let mut payload = Map::new();
        payload.insert("account".into(), json!(input.account));
        for (key, value) in [
            ("kind", &input.kind),
            ("period", &input.period),
            ("start", &input.start),
            ("end", &input.end),
            ("register", &input.register),
            ("periodicity", &input.periodicity),
        ] {
            if !value.trim().is_empty() {
                payload.insert(key.into(), json!(value));
            }
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

        let reply: QueryReply = client.tell("accounts", &Value::Object(payload))?;
        Ok(super::data::render_table(&client.base().name, &reply))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_set(tag: &str) -> Set {
        let dir = std::env::temp_dir().join(format!("gfdata-slice-{}-{tag}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("bases.json");
        let _ = std::fs::remove_file(&path);
        let mut s = Set::new("0.1.0");
        s.registry_path = Some(path);
        s
    }

    #[test]
    fn регистр_обязателен_для_среза() {
        let err = make_set("срез").slice(&SliceInput::default()).unwrap_err();
        assert_eq!(err.kind, Kind::BadRequest);
        assert!(err.to_string().contains("регистр не назван"), "{err}");
    }

    #[test]
    fn счёт_обязателен_и_отказ_говорит_где_взять_перечень() {
        let err = make_set("счёт")
            .accounts(&AccountsInput::default())
            .unwrap_err();
        assert_eq!(err.kind, Kind::BadRequest);
        assert!(
            err.to_string().contains("62.01"),
            "пример важнее правила: {err}"
        );
        assert!(err.to_string().contains("metadata"), "{err}");
    }
}
