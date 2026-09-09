//! Лояльный приём списков.
//!
//! Слабые модели заворачивают JSON-массив в строку — `"resources": "[\"СуммаОборотДт\"]"` —
//! и строгая схема отвечает сырым отказом валидатора, который модель прочитать не умеет:
//! 18 отказов одной природы за одну живую сессию 02.09.2026. Сервер строится для слабых
//! моделей, поэтому строку с массивом внутри принимает и разбирает сам.

use std::collections::BTreeMap;

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// `Vec<String>`, принимающий три формы: массив строк, строку с JSON-массивом внутри и
/// одиночную строку (список из одного элемента).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StringList(pub Vec<String>);

impl StringList {
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'de> Deserialize<'de> for StringList {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            List(Vec<String>),
            One(String),
        }

        Ok(match Raw::deserialize(d)? {
            Raw::List(v) => StringList(v),
            Raw::One(s) => {
                let trimmed = s.trim();
                if trimmed.starts_with('[') {
                    match serde_json::from_str::<Vec<String>>(trimmed) {
                        Ok(v) => StringList(v),
                        // Строка похожа на массив, но им не является — берём как есть:
                        // отказ здесь был бы придиркой к форме, а не к смыслу.
                        Err(_) => StringList(vec![s]),
                    }
                } else {
                    StringList(vec![s])
                }
            }
        })
    }
}

/// Карта параметров, принимающая две формы: сам объект и объект, завёрнутый в строку.
///
/// Форма-строка — не выдумка: GLM-5.3 в живой сессии 08.09.2026 прислал её семь раз подряд
/// и все семь получил отказ, потому что схема объявляла `parameters` строкой, а структура
/// требовала карту. Схема исправлена, но приём строки остаётся: модель, однажды завернувшая
/// JSON в строку, завернёт его снова.
///
/// Пары вида `Н=2026-01-01; К=2026-12-31` (первое, что попробовала та же модель) сознательно
/// НЕ разбираются: знак равенства и точка с запятой встречаются внутри значений, и тихо
/// неверный разбор хуже честного отказа. Отказ на такую строку называет обе принятые формы.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParamMap(pub BTreeMap<String, Value>);

impl std::ops::Deref for ParamMap {
    type Target = BTreeMap<String, Value>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Serialize for ParamMap {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

impl<'de> Deserialize<'de> for ParamMap {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Map(BTreeMap<String, Value>),
            One(String),
        }

        match Raw::deserialize(d)? {
            Raw::Map(m) => Ok(ParamMap(m)),
            Raw::One(s) => {
                let trimmed = s.trim();
                // Пустая строка — это «параметров нет», а не поломка: модель так отвечает
                // на необязательное поле чаще, чем опускает его.
                if trimmed.is_empty() {
                    return Ok(ParamMap::default());
                }
                if trimmed.starts_with('{') {
                    if let Ok(m) = serde_json::from_str::<BTreeMap<String, Value>>(trimmed) {
                        return Ok(ParamMap(m));
                    }
                }
                Err(de::Error::custom(format!(
                    "параметры даны строкой, но это не объект JSON: {s}. Принимаются две формы: объект {{\"Н\": \"2026-01-01\"}} или тот же объект целиком завёрнутый в строку. Пары через «=» и «;» не разбираются: оба знака встречаются внутри значений."
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn разобрать(json: &str) -> StringList {
        serde_json::from_str(json).expect("разбор не удался")
    }

    #[test]
    fn массив_строк_принимается() {
        assert_eq!(
            разобрать(r#"["Количество","Сумма"]"#).0,
            vec!["Количество", "Сумма"]
        );
    }

    #[test]
    fn массив_завёрнутый_в_строку_разбирается() {
        assert_eq!(
            разобрать(r#""[\"СуммаОборотДт\"]""#).0,
            vec!["СуммаОборотДт"],
            "ровно эта форма дала 18 отказов за одну живую сессию"
        );
    }

    #[test]
    fn одиночная_строка_это_список_из_одного() {
        assert_eq!(разобрать(r#""Количество""#).0, vec!["Количество"]);
    }

    #[test]
    fn пустой_массив_остаётся_пустым() {
        assert!(разобрать("[]").is_empty());
    }

    fn параметры(json: &str) -> ParamMap {
        serde_json::from_str(json).expect("разбор не удался")
    }

    #[test]
    fn параметры_объектом_принимаются() {
        let m = параметры(r#"{"Н":"2026-01-01","К":"2026-12-31"}"#);
        assert_eq!(m.len(), 2);
        assert_eq!(m["Н"], Value::from("2026-01-01"));
    }

    #[test]
    fn параметры_завёрнутые_в_строку_разбираются() {
        let m = параметры(r#""{\"Н\":\"2026-01-01\"}""#);
        assert_eq!(
            m["Н"],
            Value::from("2026-01-01"),
            "ровно эта форма дала семь отказов подряд в сессии 08.09.2026"
        );
    }

    #[test]
    fn пустая_строка_это_отсутствие_параметров() {
        assert!(параметры(r#""""#).is_empty());
    }

    #[test]
    fn пары_через_равно_отвергаются_а_не_разбираются_молча() {
        let e = serde_json::from_str::<ParamMap>(r#""Н=2026-01-01; К=2026-12-31""#)
            .expect_err("такая строка обязана быть отвергнута, а не разобрана наугад");
        let текст = e.to_string();
        assert!(
            текст.contains("две формы"),
            "отказ обязан назвать принятые формы: {текст}"
        );
    }

    #[test]
    fn параметры_переживают_обратную_сериализацию() {
        let m = параметры(r#"{"Н":"2026-01-01"}"#);
        assert_eq!(serde_json::to_string(&m).unwrap(), r#"{"Н":"2026-01-01"}"#);
    }

    #[test]
    fn похожая_на_массив_но_битая_строка_берётся_как_есть() {
        // Отказ здесь был бы придиркой к форме: пусть база скажет, что не так со смыслом.
        assert_eq!(разобрать(r#""[не json""#).0, vec!["[не json"]);
    }
}
