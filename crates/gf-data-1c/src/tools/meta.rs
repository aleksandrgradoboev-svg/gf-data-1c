//! Метаданные конфигурации: что за база, из чего она состоит, как устроен объект.
//!
//! Эти три инструмента отвечают на вопросы ДО написания запроса. Главное их свойство —
//! называть имена так, как их примет запрос, а не так, как они записаны в метаданных:
//! у виртуальных таблиц регистра имена другие, а небалансовые поля регистра бухгалтерии
//! существуют в таблице только как «имяДт» и «имяКт».

use serde::Deserialize;

use crate::refusal::{Kind, Refusal};

use super::Set;

// ── Общая информация о базе ──────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct BaseInfoInput {
    /// Имя базы 1С из реестра. Обязательно; перечень — `bases` с `action=list`.
    pub base: String,
}

pub const BASE_INFO_NAME: &str = "base_info";
pub const BASE_INFO_DESCRIPTION: &str = "Получить общую информацию о базе 1С: название \
конфигурации, версия, поставщик, платформа, режим совместимости. Используй первым делом, \
чтобы понять, с какой конфигурацией работаешь.";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BaseInfoReply {
    #[serde(rename = "конфигурация")]
    configuration: String,
    #[serde(rename = "синоним")]
    synonym: String,
    #[serde(rename = "версия")]
    version: String,
    #[serde(rename = "поставщик")]
    vendor: String,
    #[serde(rename = "платформа")]
    platform: String,
    #[serde(rename = "режимСовместимости")]
    compatibility: String,
}

// ── Состав конфигурации ──────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct MetadataInput {
    /// Имя базы 1С из реестра. Обязательно.
    pub base: String,
    /// Категория метаданных: Справочники, Документы, Перечисления, РегистрыСведений,
    /// РегистрыНакопления и др. Без фильтра приходит сводка по категориям.
    pub filter: String,
}

pub const METADATA_NAME: &str = "metadata";
pub const METADATA_DESCRIPTION: &str = "Список объектов конфигурации 1С по категориям: \
справочники, документы, регистры, перечисления и т.д. Без фильтра — сводка (категория и \
количество), с filter — полный перечень объектов категории. Вызывай первым при работе с \
незнакомой конфигурацией: имена объектов из результата используются в object и в запросах.";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CategoryCount {
    #[serde(rename = "категория")]
    category: String,
    #[serde(rename = "количество")]
    count: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct NamedItem {
    #[serde(rename = "имя")]
    name: String,
    #[serde(rename = "синоним")]
    synonym: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MetadataReply {
    #[serde(rename = "категории")]
    categories: Vec<CategoryCount>,
    #[serde(rename = "категория")]
    category: String,
    #[serde(rename = "количество")]
    count: i64,
    #[serde(rename = "объекты")]
    objects: Vec<NamedItem>,
}

// ── Структура объекта ────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ObjectInput {
    /// Имя базы 1С из реестра. Обязательно.
    pub base: String,
    /// Тип объекта: Catalog, Document, Enum, InformationRegister, AccumulationRegister,
    /// AccountingRegister, CalculationRegister, ChartOfAccounts и др.
    pub object_type: String,
    /// Имя объекта метаданных, например Номенклатура.
    pub object_name: String,
}

pub const OBJECT_NAME: &str = "object";
pub const OBJECT_DESCRIPTION: &str = "Получить поля объекта метаданных 1С — из чего он состоит \
и какими именами им пользоваться в запросе: стандартные поля платформы (Период, Регистратор, \
Активность, Номер, Дата, Проведен), реквизиты, измерения, ресурсы, табличные части, значения \
перечисления. Типы называются явно (CatalogRef.Номенклатура, Number(15,2)). Вызывай ПЕРЕД \
написанием запроса: имена ресурсов виртуальных таблиц регистра отличаются от имён самого \
регистра, а у регистра бухгалтерии небалансовые измерения и ресурсы существуют в таблице \
только как «имяДт» и «имяКт» — ответ показывает их в этом виде.";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Field {
    #[serde(rename = "имя")]
    name: String,
    #[serde(rename = "синоним")]
    synonym: String,
    #[serde(rename = "тип")]
    kind: String,
    /// Только у регистра бухгалтерии. Небалансовое измерение или ресурс раздваивается
    /// в таблице запроса на «имяДт» и «имяКт» — имя из метаданных туда не подставляется.
    /// `Option`, а не `bool`: отсутствие признака и «небалансовое» — разные вещи.
    #[serde(rename = "балансовый")]
    balanced: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AccountingFeatures {
    #[serde(rename = "корреспонденция")]
    correspondence: bool,
    #[serde(rename = "поляСчета")]
    account_fields: Vec<String>,
    #[serde(rename = "планСчетов")]
    chart: String,
    #[serde(rename = "максСубконто")]
    max_subconto: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct VirtualTable {
    #[serde(rename = "имя")]
    name: String,
    #[serde(rename = "поля")]
    fields: Vec<String>,
    #[serde(rename = "параметры")]
    params: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TablePart {
    #[serde(rename = "имя")]
    name: String,
    #[serde(rename = "синоним")]
    synonym: String,
    #[serde(rename = "реквизиты")]
    attributes: Vec<Field>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ObjectReply {
    #[serde(rename = "тип")]
    kind: String,
    #[serde(rename = "имя")]
    name: String,
    #[serde(rename = "синоним")]
    synonym: String,
    #[serde(rename = "полноеИмя")]
    full_name: String,
    /// Поля, которые заводит сама платформа: Период, Регистратор, Активность у регистров,
    /// Номер, Дата, Проведен у документов. В коллекциях метаданных их нет, а запрос
    /// пишется именно по ним.
    #[serde(rename = "стандартныеПоля")]
    standard_fields: Vec<Field>,
    #[serde(rename = "особенностиРегистраБухгалтерии")]
    accounting: Option<AccountingFeatures>,
    #[serde(rename = "реквизиты")]
    attributes: Vec<Field>,
    #[serde(rename = "измерения")]
    dimensions: Vec<Field>,
    #[serde(rename = "ресурсы")]
    resources: Vec<Field>,
    /// Виртуальные таблицы регистра с ФАКТИЧЕСКИМИ именами полей: у них имена другие,
    /// чем в основной таблице. Раньше отдавалось правило их построения, и правило нужно
    /// было применить — на этом шаге слабая модель и ошибалась (прогон 31.08.2026, три
    /// отказа из пяти). Готовый перечень применять не нужно.
    #[serde(rename = "виртуальныеТаблицы")]
    virtual_tables: Vec<VirtualTable>,
    #[serde(rename = "табличныеЧасти")]
    table_parts: Vec<TablePart>,
    #[serde(rename = "значения")]
    values: Vec<NamedItem>,
    /// Только для DefinedType — состав типов; и для Subsystem — что в неё входит.
    #[serde(rename = "типы")]
    types: Vec<String>,
    #[serde(rename = "состав")]
    content: Vec<String>,
    #[serde(rename = "подсистемы")]
    subsystems: Vec<NamedItem>,
}

impl Set {
    /// Общая информация о базе.
    pub fn base_info(&self, input: &BaseInfoInput) -> Result<String, Refusal> {
        let client = self.channel_for(&input.base)?;
        let r: BaseInfoReply = client.ask("base", &[])?;

        Ok(format!(
            "База: {}\nКонфигурация: {} ({})\nВерсия: {}\nПоставщик: {}\nПлатформа: {}\n\
             Режим совместимости: {}",
            client.base().name,
            r.configuration,
            r.synonym,
            r.version,
            r.vendor,
            r.platform,
            r.compatibility
        ))
    }

    /// Состав конфигурации: сводка по категориям или перечень объектов категории.
    pub fn metadata(&self, input: &MetadataInput) -> Result<String, Refusal> {
        let client = self.channel_for(&input.base)?;

        let filter = input.filter.trim();
        let query: Vec<(&str, &str)> = if filter.is_empty() {
            Vec::new()
        } else {
            vec![("filter", filter)]
        };

        let r: MetadataReply = client.ask("metadata", &query)?;

        if input.filter.is_empty() {
            let mut out = format!("Состав конфигурации базы {}:\n\n", client.base().name);
            let mut total = 0i64;
            for c in &r.categories {
                out.push_str(&format!("  {:<30} {}\n", c.category, c.count));
                total += c.count;
            }
            out.push_str(&format!("\nВсего объектов по категориям: {total}.\n"));
            out.push_str(
                "Перечень объектов категории: тот же инструмент с filter=\"<категория>\".",
            );
            return Ok(out);
        }

        let mut out = format!(
            "{} базы {}: {}\n\n",
            r.category,
            client.base().name,
            r.count
        );
        for o in &r.objects {
            if !o.synonym.is_empty() && o.synonym != o.name {
                out.push_str(&format!("  {} — {}\n", o.name, o.synonym));
            } else {
                out.push_str(&format!("  {}\n", o.name));
            }
        }
        Ok(out)
    }

    /// Структура объекта метаданных.
    pub fn object(&self, input: &ObjectInput) -> Result<String, Refusal> {
        if input.object_type.trim().is_empty() || input.object_name.trim().is_empty() {
            return Err(Refusal::new(
                Kind::BadRequest,
                "объект не назван",
                "нужны object_type и object_name",
            )
            .hint("перечень объектов категории — инструмент metadata с filter"));
        }

        let client = self.channel_for(&input.base)?;
        let r: ObjectReply = client
            .ask(
                "object",
                &[
                    ("type", input.object_type.as_str()),
                    ("name", input.object_name.as_str()),
                ],
            )
            .map_err(hint_not_found)?;

        let mut out = format!("{} ({})\n", r.full_name, r.synonym);

        // Раздвоение имён включается свойством регистра, а не типом поля: при
        // корреспонденции небалансовые измерения и ресурсы существуют в таблице
        // только как «имяДт» и «имяКт».
        let split = r.accounting.as_ref().is_some_and(|a| a.correspondence);

        write_fields(&mut out, "Стандартные поля", &r.standard_fields, false);
        write_fields(&mut out, "Реквизиты", &r.attributes, false);
        write_fields(&mut out, "Измерения", &r.dimensions, split);
        write_fields(&mut out, "Ресурсы", &r.resources, split);

        for part in &r.table_parts {
            write_fields(
                &mut out,
                &format!("Табличная часть {}", part.name),
                &part.attributes,
                false,
            );
        }

        if let Some(a) = &r.accounting {
            out.push_str("\nРегистр бухгалтерии:\n");
            out.push_str(&format!(
                "  счёт в запросе — {}\n",
                a.account_fields.join(", ")
            ));
            if !a.chart.is_empty() {
                out.push_str(&format!("  план счетов — ПланСчетов.{}", a.chart));
                if a.max_subconto > 0 {
                    out.push_str(&format!(", субконто до {}", a.max_subconto));
                }
                out.push('\n');
            }
            if split {
                out.push_str(
                    "  небалансовые измерения и ресурсы показаны выше в том виде, в каком \
                     существуют в таблице: «имяДт» и «имяКт»\n",
                );
            }
            out.push_str(
                "  отбор по счёту-группе через «=» вернёт ноль строк без ошибки — пиши \
                 «В ИЕРАРХИИ (&Счет)»\n",
            );
            out.push_str(
                "  Субконто1..N и ВидСубконто1..N платформа объявляет стандартными полями, но в \
                 ОСНОВНОЙ таблице регистра их нет — запрос по ним не разберётся. Они лежат в \
                 виртуальной ДвиженияССубконто(Начало, Конец, Условие, Порядок, \
                 МаксимальноеКоличество) и называются там СубконтоДт1..N и СубконтоКт1..N; \
                 условие идёт ТРЕТЬИМ параметром\n",
            );
        }

        if !r.values.is_empty() {
            out.push_str(&format!("\nЗначения перечисления ({}):\n", r.values.len()));
            for v in &r.values {
                out.push_str(&format!("  {} — {}\n", v.name, v.synonym));
            }
        }
        if !r.types.is_empty() {
            out.push_str(&format!(
                "\nСостав определяемого типа ({}):\n",
                r.types.len()
            ));
            for t in &r.types {
                out.push_str(&format!("  {t}\n"));
            }
        }
        if !r.content.is_empty() {
            out.push_str(&format!("\nСостав подсистемы ({}):\n", r.content.len()));
            for o in &r.content {
                out.push_str(&format!("  {o}\n"));
            }
        }
        if !r.subsystems.is_empty() {
            out.push_str(&format!(
                "\nВложенные подсистемы ({}):\n",
                r.subsystems.len()
            ));
            for s in &r.subsystems {
                out.push_str(&format!("  {} — {}\n", s.name, s.synonym));
            }
        }

        if !r.virtual_tables.is_empty() {
            // Готовый перечень вместо правила его построения: правило нужно ПРИМЕНИТЬ, а
            // именно на применении слабая модель и ошибается — в прогоне 31.08.2026 три
            // отказа из пяти были обращением к Обороты.СчетДт и ОборотыДтКт.СуммаОборотДт,
            // которых у этих таблиц нет. Из списка достаточно выбрать.
            out.push_str(
                "\nВиртуальные таблицы регистра — ИМЕНА ПОЛЕЙ ЗДЕСЬ ДРУГИЕ, чем в основной;\n\
                 берите их отсюда, а не достраивайте по образцу:\n",
            );
            for vt in &r.virtual_tables {
                out.push_str(&format!("\n  {}.{}", r.full_name, vt.name));
                if !vt.params.is_empty() {
                    out.push_str(&format!("({})", vt.params.join(", ")));
                }
                out.push_str(&format!("\n    поля: {}\n", vt.fields.join(", ")));
            }
        } else if !r.resources.is_empty() {
            out.push_str(
                "\nВ виртуальных таблицах регистра имена ресурсов другие: к остаткам добавляется \
                 «Остаток», к оборотам — «Оборот».",
            );
        }

        let _ = r.kind;
        let _ = r.name;
        Ok(out)
    }
}

/// `split` — раздваивать ли имена небалансовых полей на «имяДт» / «имяКт». Печатается
/// именно то имя, которое примет запрос: имя из метаданных здесь ввело бы в заблуждение
/// молча.
fn write_fields(out: &mut String, title: &str, fields: &[Field], split: bool) {
    if fields.is_empty() {
        return;
    }
    out.push_str(&format!("\n{title} ({}):\n", fields.len()));
    for f in fields {
        let name = if split && f.balanced == Some(false) {
            format!("{}Дт / {}Кт", f.name, f.name)
        } else {
            f.name.clone()
        };
        out.push_str(&format!("  {name:<40} {}\n", f.kind));
        let _ = &f.synonym;
    }
}

/// Добавляет отказу «объект не найден» ход, которым его надо закрывать.
///
/// Заведено по случаю 26.08.2026: отказ прочитался как факт о 1С вообще, и модель
/// принялась достраивать имя документа по образцу — «может, он называется иначе». Отказ
/// теперь называет базу сам, а здесь дописывается, куда идти вместо подбора имён.
///
/// Опрашивать соседние базы — «а нет ли объекта там» — сознательно НЕ делаем. Соседняя
/// база это другая конфигурация и, как правило, другая организация: подсказка звала бы за
/// чужими данными, а отчёт по ним вышел бы складным и неверным по смыслу. Плюс цена: по
/// HTTP-запросу на базу с полным таймаутом, и всё это на пути отказа, который обязан быть
/// быстрым.
fn hint_not_found(err: Refusal) -> Refusal {
    if err.kind != Kind::BaseError || !err.what.contains("не найден") {
        return err;
    }
    err.hint(
        "это отказ по НАЗВАННОЙ базе, а не про 1С вообще: в другой конфигурации объект может \
         называться иначе или отсутствовать",
    )
    .hint("перечень объектов категории — metadata с filter; подбирать имя по образцу нельзя")
    .hint(
        "если объект из другой конфигурации — назовите нужную базу параметром base \
         (перечень: bases с action=list)",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_set(tag: &str) -> Set {
        let dir = std::env::temp_dir().join(format!("gfdata-meta-{}-{tag}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("bases.json");
        let _ = std::fs::remove_file(&path);
        let mut s = Set::new("0.1.0");
        s.registry_path = Some(path);
        s
    }

    #[test]
    fn объект_без_имени_даёт_отказ_до_обращения_к_базе() {
        let s = make_set("без-имени");
        let err = s
            .object(&ObjectInput {
                base: "ut11".into(),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(err.kind, Kind::BadRequest);
        assert!(
            err.to_string().contains("object_type и object_name"),
            "{err}"
        );
    }

    #[test]
    fn база_обязательна_у_всех_трёх() {
        let s = make_set("без-базы");
        for err in [
            s.base_info(&BaseInfoInput::default()).unwrap_err(),
            s.metadata(&MetadataInput::default()).unwrap_err(),
            s.object(&ObjectInput {
                object_type: "Catalog".into(),
                object_name: "Номенклатура".into(),
                ..Default::default()
            })
            .unwrap_err(),
        ] {
            assert_eq!(err.kind, Kind::BadRequest, "{err}");
            assert!(err.to_string().contains("база не названа"), "{err}");
        }
    }

    #[test]
    fn отказ_не_найден_зовёт_к_metadata_а_не_к_подбору_имени() {
        let err = hint_not_found(Refusal::new(
            Kind::BaseError,
            "объект не найден",
            "Документ.Чепуха",
        ));
        let text = err.to_string();
        assert!(text.contains("подбирать имя по образцу нельзя"), "{text}");
        assert!(text.contains("а не про 1С вообще"), "{text}");
    }

    #[test]
    fn чужой_отказ_подсказкой_не_обрастает() {
        let err = hint_not_found(Refusal::new(
            Kind::Unauthorized,
            "база отказала в доступе",
            "HTTP 401",
        ));
        assert!(
            err.hints.is_empty(),
            "подсказка про metadata здесь была бы не к месту"
        );
    }

    #[test]
    fn небалансовые_поля_печатаются_как_их_примет_запрос() {
        let mut out = String::new();
        write_fields(
            &mut out,
            "Измерения",
            &[
                Field {
                    name: "Организация".into(),
                    kind: "CatalogRef.Организации".into(),
                    balanced: Some(true),
                    ..Default::default()
                },
                Field {
                    name: "Подразделение".into(),
                    kind: "CatalogRef.Подразделения".into(),
                    balanced: Some(false),
                    ..Default::default()
                },
            ],
            true,
        );
        assert!(
            out.contains("Организация "),
            "балансовое имя не раздваивается: {out}"
        );
        assert!(
            out.contains("ПодразделениеДт / ПодразделениеКт"),
            "небалансовое существует в таблице только так: {out}"
        );
    }

    #[test]
    fn без_корреспонденции_имена_не_раздваиваются() {
        let mut out = String::new();
        write_fields(
            &mut out,
            "Измерения",
            &[Field {
                name: "Подразделение".into(),
                kind: "CatalogRef.Подразделения".into(),
                balanced: Some(false),
                ..Default::default()
            }],
            false,
        );
        assert!(!out.contains("Дт /"), "{out}");
    }

    #[test]
    fn пустой_список_полей_не_печатает_заголовок() {
        let mut out = String::new();
        write_fields(&mut out, "Ресурсы", &[], false);
        assert!(
            out.is_empty(),
            "пустой раздел читается как поломка: {out:?}"
        );
    }
}
