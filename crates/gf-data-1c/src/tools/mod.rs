//! Инструменты сервера: то, что вызывает агент.
//!
//! Каждый инструмент — обычная функция, принимающая разобранные аргументы и
//! возвращающая либо готовый текст ответа, либо отказ. Про MCP они не знают ничего:
//! протокол живёт снаружи, а линкующему приложению его и вовсе не нужно. Ровно это
//! разделение и делает продукт пригодным для встраивания.

pub mod bases;
pub mod gate;
pub mod meta;
pub mod probe;
pub mod queryhints;

use std::path::PathBuf;
use std::time::Duration;

use crate::channel::Client;
use crate::refusal::Refusal;
use crate::registry::Registry;

/// Набор инструментов, разделяющих общее состояние: путь реестра и таймаут канала.
///
/// Реестр читается на каждый вызов, а не кэшируется: его правят снаружи (в том числе
/// инструментом `bases`), и агент не должен видеть устаревший список.
pub struct Set {
    pub registry_path: Option<PathBuf>,
    pub timeout: Option<Duration>,
    /// Версия сервера. Нужна пробе: расширение старше сервера отвечает не ошибкой,
    /// а пустотой в новых методах, и это неотличимо от отсутствия данных.
    pub version: String,
    /// Отключает гейт построителя (см. [`gate`]): `query` выполняет любой текст, а
    /// `query_check` не запирается после отказа. Только для тестов, которые проверяют
    /// сам язык и канал, и для вызывающей стороны, которая не является языковой
    /// моделью; в поставке гейт включён всегда — его нельзя выключить вызовом.
    pub allow_raw_query: bool,
    /// Состояние сессии для `query_check` / `query` / `query_build`.
    pub gate: gate::QueryGate,
}

impl Set {
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            registry_path: None,
            timeout: None,
            version: version.into(),
            allow_raw_query: false,
            gate: gate::QueryGate::new(),
        }
    }

    pub fn registry(&self) -> Result<Registry, Refusal> {
        Registry::load(self.registry_path.as_deref())
    }

    /// Открывает канал к названной базе, разрешая имя по реестру.
    pub fn channel_for(&self, name: &str) -> Result<Client, Refusal> {
        let reg = self.registry()?;
        let base = reg.resolve(name)?;
        Client::new(base, self.timeout)
    }
}
