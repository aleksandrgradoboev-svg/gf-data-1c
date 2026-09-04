//! Проба канала: отвечают ли базы, и если нет — почему именно.
//!
//! Инструмент намеренно НЕ возвращает отказ, когда база не ответила: неответ базы —
//! это его результат, ради которого его и звали. Отказом заканчивается только
//! невозможность выполнить саму проверку (нечего проверять, реестр не читается).

use serde::Deserialize;

use crate::channel::Client;
use crate::refusal::{Kind, Refusal};
use crate::registry::Base;

use super::Set;

/// Параметры диагностики канала.
///
/// `base` здесь единственный на весь сервер необязательный: у остальных инструментов
/// данных он обязателен, потому что пустое значение означало бы «выбери базу за меня»
/// и тихо увело бы вызов не туда. У пробы пустое значение значит противоположное —
/// «проверь все», и ответ перечисляет базы поимённо. Опасности умолчания тут нет по
/// устройству: молчаливого выбора не происходит.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ProbeInput {
    pub base: String,
}

pub const NAME: &str = "probe";

pub const DESCRIPTION: &str = "Проверить, отвечают ли базы 1С из реестра: по каждой базе \
отдельная строка — жив канал, не установлено расширение (404), не поднят веб-сервер \
(соединение отвергнуто) или отказ прав (401). Вызывай перед работой с данными и всякий раз, \
когда база отвечает пусто: пустой ответ мёртвого канала выглядит точно так же, как честное \
отсутствие данных. Без параметра base проверяет все базы разом.";

/// Ответ пробы. Разбирается, а не печатается сырьём: сырой JSON в строке отчёта
/// читается как поломка, даже когда всё в порядке.
#[derive(Debug, Default, Deserialize)]
struct VersionReply {
    #[serde(rename = "расширение", default)]
    extension: String,
    #[serde(rename = "платформа", default)]
    platform: String,
}

impl Set {
    /// Опрашивает базы и печатает по строке на каждую.
    pub fn probe(&self, input: &ProbeInput) -> Result<String, Refusal> {
        let reg = self.registry()?;
        if reg.bases.is_empty() {
            return Err(
                Refusal::new(Kind::BadRequest, "проверять нечего", "реестр баз пуст")
                    .hint("добавьте базу: bases с action=add"),
            );
        }

        let targets: Vec<Base> = if input.base.trim().is_empty() {
            reg.bases.clone()
        } else {
            vec![reg.resolve(&input.base)?]
        };

        let mut out = format!("Проверка канала, баз: {}\n\n", targets.len());
        let (mut alive, mut stale) = (0usize, 0usize);

        for base in &targets {
            let client = Client::new(base.clone(), self.timeout)?;
            let (mut line, version, ok) = probe_one(&client);
            if ok {
                alive += 1;
                // Расширение старше сервера отвечает на новые методы пустотой, а не
                // ошибкой: без этой сверки рассинхрон читается как «в базе ничего нет».
                if !version.is_empty() && !self.version.is_empty() && version != self.version {
                    stale += 1;
                    line.push_str(&format!(
                        "\n   ⚠ расширение {}, сервер {} — переустановите расширение в этой базе: \
                         разошедшиеся версии дают пустые ответы вместо ошибок",
                        version, self.version
                    ));
                }
            }
            out.push_str(&line);
            out.push('\n');
        }

        out.push_str(&format!("\nЖивых каналов: {} из {}.", alive, targets.len()));
        if stale > 0 {
            out.push_str(&format!(
                "\nРазошлись версии расширения и сервера: баз {stale}. \
                 Пока версии не сведены, ответам этих баз доверять нельзя."
            ));
        }
        if alive < targets.len() {
            out.push_str(
                "\nПока канал не жив, работа по данным этой базы не начинается: пустой ответ \
                 мёртвого канала неотличим от отсутствия данных.",
            );
        }
        if alive > 0 {
            out.push_str(QUERY_PRIMER);
        }
        Ok(out)
    }
}

/// Опрашивает одну базу: строка отчёта, версия расширения, признак живости.
fn probe_one(client: &Client) -> (String, String, bool) {
    let name = client.base().name.clone();
    match client.ask::<VersionReply>("version", &[]) {
        Ok(v) => (
            format!(
                "✅ {:<10} жив — расширение {}, платформа {}",
                name, v.extension, v.platform
            ),
            v.extension,
            true,
        ),
        Err(e) => {
            let line = match e.kind {
                Kind::NoWebServer => format!("❌ {name:<10} веб-сервер не отвечает — {}", e.why),
                Kind::NoPublication => {
                    format!("❌ {name:<10} базы нет по этому адресу — {}", e.why)
                }
                Kind::NoExtension => format!("❌ {name:<10} расширение не отвечает — {}", e.why),
                Kind::Unauthorized => format!("❌ {name:<10} отказ прав — {}", e.why),
                _ => format!("❌ {name:<10} {} — {}", e.what, e.why),
            };
            (line, String::new(), false)
        }
    }
}

/// Памятка по языку запросов, которая едет вместе с ответом пробы.
///
/// Доставка здесь принудительная, и это осознанно. Справка отдельным инструментом не
/// работает на слабом исполнителе: чтобы её спросить, надо усомниться, а ошибочная
/// конструкция изнутри ощущается точно так же, как верная.
const QUERY_PRIMER: &str = "

== Прежде чем писать запрос ==
Готовые инструменты считают то же самое БЕЗ языка запросов, ошибиться синтаксисом в них нельзя:
  gf-data-1c_accounts - остатки и обороты по счёту (помесячно, по субсчетам)
  gf-data-1c_register - итоги регистра накопления
  gf-data-1c_slice    - регистр сведений на дату
  gf-data-1c_count    - сколько записей
Имена даны с префиксом сервера, как их видит харнесс. Если у вас сервер зарегистрирован под
другим именем - берите имя из своего перечня инструментов, а не сокращайте до accounts: короткое
имя даёт отказ \"unavailable tool\", и он читается как поломка, хотя это опечатка.
Запрос нужен, когда готового не хватает: соединения, нетиповые разрезы, расчёт внутри запроса.

== Если всё же пишете запрос ==
  начинается с        Код ПОДОБНО \"96%\"             (НАЧАТОС, CONTAINS, LIKE - таких нет)
  пусто               Поле ЕСТЬ NULL                (ЕСТЬ ПУСТО, IS NULL - таких нет)
  конкретное значение = ЗНАЧЕНИЕ(ПланСчетов.Хозрасчетный.Имя)
  различные           КОЛИЧЕСТВО(РАЗЛИЧНЫЕ Поле)    (КОЛИЧЕСТВОРАЗЛИЧНЫХ - такого нет)
  месяц из даты       НАЧАЛОПЕРИОДА(Период, МЕСЯЦ)  (ПЕРИОД(), MONTH() - таких нет)
  соединение          ЛЕВОЕ СОЕДИНЕНИЕ ... ПО ...   (JOIN, ПОДСОЕДИНЕНИЕ - таких нет)
  план счетов         ПланСчетов.Хозрасчетный       (без префикса Справочник)
ОстаткиИОбороты(Начало, Конец, Периодичность, МетодДополнения, Условие, Субконто, Порядок):
  поля - Счет, Период, СуммаНачальныйОстатокДт/Кт, СуммаОборотДт/Кт, СуммаКонечныйОстатокДт/Кт.
  Регистратора и СчетДт в ней НЕТ - это поля основной таблицы регистра.
Полная страница по любой теме - gf-data-1c_syntax: 52 тысячи страниц справки вендора для
вашей версии платформы, включая поля и параметры всех виртуальных таблиц.

== Чего не делать никогда ==
Не складывайте числа глазами по общей выдаче, если запрос не задался: посчитанное вручную
неотличимо на вид от посчитанного базой, а сверка по нему сходится сама с собой.";

#[cfg(test)]
mod tests {
    use super::*;

    fn make_set(tag: &str) -> Set {
        let dir = std::env::temp_dir().join(format!("gfdata-probe-{}-{tag}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("bases.json");
        let _ = std::fs::remove_file(&path);
        let mut s = Set::new("0.1.0");
        s.registry_path = Some(path);
        s
    }

    #[test]
    fn пустой_реестр_даёт_отказ_а_не_пустой_отчёт() {
        let err = make_set("пусто").probe(&ProbeInput::default()).unwrap_err();
        assert_eq!(err.kind, Kind::BadRequest);
        assert!(err.to_string().contains("проверять нечего"), "{err}");
    }

    #[test]
    fn незнакомая_база_даёт_отказ_с_перечнем() {
        let s = make_set("незнакомая");
        {
            let mut reg = s.registry().unwrap();
            reg.add(Base {
                name: "ut11".into(),
                url: "http://127.0.0.1:9/hs/gt-data".into(),
                ..Default::default()
            })
            .unwrap();
        }
        let err = s
            .probe(&ProbeInput {
                base: "нетбазы".into(),
            })
            .unwrap_err();
        assert_eq!(err.kind, Kind::UnknownBase);
    }

    #[test]
    fn мёртвая_база_это_результат_пробы_а_не_отказ() {
        let s = make_set("мёртвая");
        {
            let mut reg = s.registry().unwrap();
            reg.add(Base {
                name: "мертвец".into(),
                // Порт 9 (discard) — заведомо никто не слушает.
                url: "http://127.0.0.1:9/hs/gt-data".into(),
                ..Default::default()
            })
            .unwrap();
        }
        let out = s
            .probe(&ProbeInput::default())
            .expect("проба не должна отказывать");
        assert!(out.contains("❌"), "{out}");
        assert!(out.contains("веб-сервер не отвечает"), "{out}");
        assert!(
            out.contains("Живых каналов: 0 из 1"),
            "счётчик обязан быть честным: {out}"
        );
        assert!(
            out.contains("работа по данным этой базы не начинается"),
            "{out}"
        );
        assert!(
            !out.contains("Прежде чем писать запрос"),
            "памятка едет только когда есть живой канал: {out}"
        );
    }
}
