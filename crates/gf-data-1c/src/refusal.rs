//! Главное правило продукта: отказ инструмента никогда не выглядит как пустой результат.
//!
//! Пустой ответ мёртвого канала неотличим от честного «в базе ничего нет», поэтому
//! каждый неуспех оформляется отказом, первая строка которого прямо говорит, что вызов
//! не выполнен, и по чьей вине. Отказ характеризует ВЫЗОВ, а не содержимое базы.

use std::fmt;

/// Вид отказа. Различаются те виды, которые требуют разных действий человека.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Веб-сервер не поднят: соединение отвергнуто.
    NoWebServer,
    /// Базы нет на веб-сервере: 404 страницей самого веб-сервера.
    NoPublication,
    /// Расширение не установлено в базе: 404 от самой 1С.
    NoExtension,
    /// Отказ прав: 401/403.
    Unauthorized,
    /// Базы нет в реестре.
    UnknownBase,
    /// Вызывающий передал негодные аргументы.
    BadRequest,
    /// База ответила ошибкой на осмысленный запрос.
    BaseError,
    /// Наша собственная поломка.
    Internal,
}

/// Отказ с причиной и подсказкой, что делать.
///
/// `base` — имя базы, к которой относится отказ. Без него «в этой конфигурации такого
/// нет» читается как факт о 1С вообще, и вызывающий начинает перебирать имена объекта
/// вместо того, чтобы усомниться в базе. Ровно этот сценарий и наблюдался 26.08.2026.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub kind: Kind,
    /// База, к которой относится отказ. Пусто — отказ не про конкретную базу.
    pub base: String,
    /// Что не удалось сделать.
    pub what: String,
    /// Чем именно ответила та сторона.
    pub why: String,
    /// Что предпринять вызывающему.
    pub hints: Vec<String>,
}

impl Refusal {
    pub fn new(kind: Kind, what: impl Into<String>, why: impl Into<String>) -> Self {
        Self {
            kind,
            base: String::new(),
            what: what.into(),
            why: why.into(),
            hints: Vec::new(),
        }
    }

    /// Подсказки строятся цепочкой, чтобы вызов читался как предложение:
    /// `Refusal::new(...).hint("перечень баз — bases с action=list")`.
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hints.push(hint.into());
        self
    }

    /// Проставляет базу отказу, **если она ещё не названа**. Отказ, уже знающий свою
    /// базу, не переписывается: ближе к месту ошибки известно точнее.
    #[must_use]
    pub fn stamp(mut self, base: &str) -> Self {
        if self.base.is_empty() {
            self.base = base.to_string();
        }
        self
    }

    /// Незнакомое имя базы обязано отвечать перечнем известных, иначе вызывающий
    /// решит, что база пуста.
    pub fn unknown_base(name: &str, known: &[String]) -> Self {
        let why = if known.is_empty() {
            "реестр баз пуст".to_string()
        } else {
            format!("в реестре её нет; известны: {}", known.join(", "))
        };
        Self::new(Kind::UnknownBase, format!("база {name:?} не найдена"), why)
            .hint("перечень баз — инструмент bases с action=list")
            .hint("добавить базу — bases с action=add, url и учётными данными")
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ОТКАЗ")?;
        if !self.base.is_empty() {
            // База называется в самой первой строке, а не сноской внизу: отказ,
            // прочитанный до конца, — не то же самое, что отказ, прочитанный до
            // первой точки.
            write!(f, " (база {})", self.base)?;
        }
        write!(f, ": {}", self.what)?;
        if !self.why.is_empty() {
            write!(f, " — {}", self.why)?;
        }
        f.write_str(".\n")?;
        f.write_str("Это отказ вызова, а не ответ базы: считать его отсутствием данных нельзя.")?;
        for h in &self.hints {
            write!(f, "\n• {h}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Refusal {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn отказ_начинается_со_слова_отказ() {
        let r = Refusal::new(Kind::BaseError, "запрос не выполнен", "поле не найдено");
        let text = r.to_string();
        assert!(
            text.starts_with("ОТКАЗ"),
            "первое слово решает, прочтут ли остальное: {text}"
        );
    }

    #[test]
    fn база_называется_в_первой_строке() {
        let r = Refusal::new(Kind::BaseError, "запрос не выполнен", "").stamp("ut11");
        let первая = r.to_string().lines().next().unwrap().to_string();
        assert!(
            первая.contains("ut11"),
            "имя базы обязано попасть в первую строку, а не в сноску: {первая}"
        );
    }

    #[test]
    fn отказ_говорит_что_он_про_вызов_а_не_про_данные() {
        let r = Refusal::new(
            Kind::NoWebServer,
            "канал недоступен",
            "соединение отвергнуто",
        );
        assert!(
            r.to_string().contains("не ответ базы"),
            "без этой строки пустой ответ читается как факт о базе"
        );
    }

    #[test]
    fn stamp_не_переписывает_уже_названную_базу() {
        let r = Refusal::new(Kind::BaseError, "что-то", "почему-то")
            .stamp("bu3")
            .stamp("ut11");
        assert_eq!(r.base, "bu3", "ближе к месту ошибки известно точнее");
    }

    #[test]
    fn незнакомая_база_перечисляет_известные() {
        let known = vec!["ut11".to_string(), "bu3".to_string()];
        let text = Refusal::unknown_base("нетбазы", &known).to_string();
        assert!(text.contains("ut11") && text.contains("bu3"), "{text}");
    }

    #[test]
    fn пустой_реестр_говорит_что_он_пуст() {
        let text = Refusal::unknown_base("любая", &[]).to_string();
        assert!(
            text.contains("реестр баз пуст"),
            "перечень «известны: » без имён выглядит как поломка: {text}"
        );
    }

    #[test]
    fn подсказки_идут_отдельными_строками() {
        let r = Refusal::new(Kind::BadRequest, "аргумент негоден", "")
            .hint("первое")
            .hint("второе");
        let text = r.to_string();
        assert!(
            text.contains("\n• первое") && text.contains("\n• второе"),
            "{text}"
        );
    }
}
