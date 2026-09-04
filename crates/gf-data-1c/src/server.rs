//! MCP-сервер: инструкции агенту, реестр инструментов, диспетчер вызовов.
//!
//! Транспорта здесь нет — он снаружи (`main` для stdio, `http` для Streamable HTTP).
//! Здесь только то, что одинаково для обоих: как сервер представляется, какие
//! инструменты объявляет и как вызов доходит до `tools::Set`.

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::journal;
use crate::proto;
use crate::refusal::Refusal;
use crate::tools::{self, Set};

/// Версия продукта. Расширение в базе сверяет её со своей.
pub const VERSION: &str = "0.1.0";

/// Имя сервера, как его видит клиент.
pub const SERVER_NAME: &str = "gf-data-1c";

/// То, что задаётся при запуске.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub registry_path: Option<std::path::PathBuf>,
    pub timeout: Option<std::time::Duration>,
    /// Снять гейт построителя (см. `tools::gate`). Для тестов; в `main` не задаётся.
    pub allow_raw_query: bool,
}

/// Объявление инструмента: то, что клиент видит в `tools/list`.
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    /// Схема входа в виде JSON-текста. Текстом, а не собранным `Value`: она
    /// перенесена из Go дословно и не должна пересобираться при каждом вызове.
    pub schema: &'static str,
}

/// То, что сервер говорит агенту при подключении.
///
/// Этот текст влияет на поведение агента сильнее описаний отдельных инструментов:
/// именно здесь сказано, что отказ нельзя читать как отсутствие данных.
pub const INSTRUCTIONS: &str = r#"Сервер читает конфигурацию и данные информационных баз 1С:Предприятие.
Читай ответы буквально: чего в ответе нет, того сервер не утверждал.

Баз несколько. Их список отдаёт bases (action=list), нужная называется параметром base.
base ОБЯЗАТЕЛЕН у каждого инструмента данных, и базы по умолчанию у сервера нет: вызов
без base не выполняется, а отвечает отказом с перечнем баз. Незнакомое имя базы тоже
даёт отказ с перечнем известных, а не пустой результат.

Единственное исключение — probe: там пустой base значит «проверить все базы», и ответ
называет каждую поимённо.

Отсюда же читай отказ «объект не найден»: он говорит про НАЗВАННУЮ базу, а не про 1С
вообще. Если объект есть в соседней базе реестра, отказ скажет и это. Перебирать имена
объекта, пока какое-нибудь не найдётся, — неверный ход: сначала проверь, та ли база.

Ответ, начинающийся словом ОТКАЗ, говорит о ВЫЗОВЕ, а не о содержимом базы: запрошенное
не выполнено. Прежде чем считать пустой ответ фактом, проверь канал инструментом probe —
он различает погашенный веб-сервер, неустановленное расширение и отказ прав.

Текст запроса пишет платформа, а не ты. query выполняет ТОЛЬКО текст, который в этой сессии
вернул query_build: назови источник, поля (строками или {поле, функция, как}), отбор,
группировку, порядок — и выполни собранный текст как есть. Написанный руками текст query не
выполняет, даже прошедший query_check; тот остаётся диагностикой синтаксиса, и после одного
его отказа следующий текст не разбирается, пока не позван query_build. Соединения и пакеты
построитель не собирает — такой вопрос возвращается словами «не собирается», без обходного
текста и без счёта вручную по общей выдаче."#;

/// Сервер: набор инструментов и состояние сессии.
///
/// Гейт построителя живёт в `Set` и потому привязан к экземпляру сервера. Это и есть
/// граница сессии: у каждого подключения свой сервер, а значит своя история проверок
/// текста запроса.
pub struct Server {
    set: Set,
}

impl Server {
    pub fn new(opts: Options) -> Self {
        let mut set = Set::new(VERSION);
        set.registry_path = opts.registry_path;
        set.timeout = opts.timeout;
        set.allow_raw_query = opts.allow_raw_query;
        Server { set }
    }

    /// Обрабатывает одно сообщение JSON-RPC.
    ///
    /// Возвращает готовый ответ либо `None` для уведомлений — им отвечать не положено.
    pub fn handle(&mut self, текст: &str) -> Option<Value> {
        let запрос: proto::Request = match serde_json::from_str(текст) {
            Ok(r) => r,
            Err(e) => {
                return Some(proto::err(
                    Value::Null,
                    proto::ErrorCode::ParseError,
                    format!("сообщение не разобрано: {e}"),
                ))
            }
        };
        // Уведомление: ответа не ждут. Отвечать на него — нарушение протокола.
        let id = запрос.id.clone()?;

        let результат = match запрос.method.as_str() {
            "initialize" => {
                let версия = запрос.params["protocolVersion"].as_str();
                Ok(proto::initialize_result(
                    SERVER_NAME,
                    VERSION,
                    proto::согласовать_версию(версия),
                ))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools_list() })),
            "tools/call" => self.call_tool(&запрос.params),
            другой => {
                return Some(proto::err(
                    id,
                    proto::ErrorCode::MethodNotFound,
                    format!("метод {другой} не обслуживается"),
                ))
            }
        };

        Some(match результат {
            Ok(v) => proto::ok(id, v),
            Err((code, msg)) => proto::err(id, code, msg),
        })
    }

    /// Вызов инструмента.
    ///
    /// Отказ инструмента — это УСПЕШНЫЙ ответ протокола с `isError: true`, а не ошибка
    /// JSON-RPC: клиент обязан различать «сервер сломался» и «вызов не удался».
    fn call_tool(&mut self, params: &Value) -> Result<Value, (proto::ErrorCode, String)> {
        let Some(имя) = params["name"].as_str() else {
            return Err((
                proto::ErrorCode::InvalidParams,
                "не назван инструмент: нужно поле name".into(),
            ));
        };
        let аргументы = params.get("arguments").cloned().unwrap_or(json!({}));

        let начало = std::time::Instant::now();
        let итог = self.dispatch(имя, &аргументы);
        let потрачено = начало.elapsed();

        // Журнал вызовов пишется здесь, а не в транспорте: он должен видеть все вызовы
        // независимо от того, каким транспортом они пришли.
        if journal::enabled() {
            journal::writef(format_args!(
                "вызов {имя} {} за {} → {}",
                mask_arguments(&аргументы),
                format_duration(потрачено),
                outcome(&итог)
            ));
        }

        match итог {
            Ok(текст) => Ok(proto::tool_result(текст, false)),
            Err(ToolError::Unknown(имя)) => Err((
                proto::ErrorCode::InvalidParams,
                format!("нет инструмента {имя}; перечень — tools/list"),
            )),
            Err(ToolError::BadArguments(e)) => Err((
                proto::ErrorCode::InvalidParams,
                format!("аргументы не разобраны: {e}"),
            )),
            Err(ToolError::Refused(r)) => Ok(proto::tool_result(r.to_string(), true)),
        }
    }

    /// Разбирает аргументы под нужный инструмент и зовёт его.
    fn dispatch(&mut self, имя: &str, args: &Value) -> Result<String, ToolError> {
        /// Разбор аргументов в тип инструмента.
        fn arg<T: DeserializeOwned>(v: &Value) -> Result<T, ToolError> {
            serde_json::from_value(v.clone()).map_err(|e| ToolError::BadArguments(e.to_string()))
        }
        let s = &mut self.set;
        let итог = match имя {
            "bases" => s.bases(&arg(args)?),
            "probe" => s.probe(&arg(args)?),
            "base_info" => s.base_info(&arg(args)?),
            "metadata" => s.metadata(&arg(args)?),
            "object" => s.object(&arg(args)?),
            "query_check" => s.query_check(&arg(args)?),
            "query_parse" => s.query_parse(&arg(args)?),
            "query_build" => s.query_build(&arg(args)?),
            "query" => s.query(&arg(args)?),
            "count" => s.count(&arg(args)?),
            "register" => s.register(&arg(args)?),
            "slice" => s.slice(&arg(args)?),
            "accounts" => s.accounts(&arg(args)?),
            "export" => s.export(&arg(args)?),
            "syntax" => s.syntax(&arg(args)?),
            "eventlog" => s.event_log(&arg(args)?),
            другой => return Err(ToolError::Unknown(другой.to_string())),
        };
        итог.map_err(ToolError::Refused)
    }
}

/// Чем кончился вызов инструмента.
enum ToolError {
    /// Такого инструмента нет — ошибка протокола, а не отказ инструмента.
    Unknown(String),
    /// Аргументы не разобрались — тоже уровень протокола.
    BadArguments(String),
    /// Инструмент отработал и говорит, что не вышло.
    Refused(Refusal),
}

/// Перечень инструментов для `tools/list`.
pub fn tools_list() -> Vec<Value> {
    TOOLS
        .iter()
        .map(|t| {
            // Схема query_build собирается кодом, а не берётся из таблицы: она
            // принимает лояльные формы (строка вместо массива, массив, завёрнутый
            // в строку, русские ключи через additionalProperties), которых выведенная
            // из полей схема не знает. Валидатор клиента проверяет вызов ДО обработчика,
            // и строгая схема отвергала бы ровно те формы, ради приёма которых
            // построитель и заведён.
            let схема: Value = if t.name == "query_build" {
                tools::queryschema::query_build_schema()
            } else {
                serde_json::from_str(t.schema).unwrap_or_else(|_| json!({}))
            };
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": схема,
            })
        })
        .collect()
}

/// Итог вызова одной строкой — для журнала.
///
/// Отказ инструмента приходит результатом, а не ошибкой, и в журнале эти два случая
/// надо различать: первый — ответ сервера, второй — поломка.
fn outcome(итог: &Result<String, ToolError>) -> String {
    match итог {
        Ok(текст) => {
            let head = shorten(текст);
            if head.is_empty() {
                "ок".into()
            } else {
                format!("ок: {head}")
            }
        }
        Err(ToolError::Refused(r)) => format!("ОТКАЗ: {}", shorten(&r.to_string())),
        Err(ToolError::Unknown(имя)) => format!("СБОЙ: нет инструмента {имя}"),
        Err(ToolError::BadArguments(e)) => format!("СБОЙ: аргументы не разобраны: {}", shorten(e)),
    }
}

/// Сколько символов итога писать в журнал.
///
/// Ответы бывают в десятки килобайт, а для разбора нужна опознавательная строка,
/// а не весь текст.
const JOURNAL_LIMIT: usize = 200;

/// Поля, значения которых в журнал не идут ни при каких условиях.
const SECRET_KEYS: &[&str] = &["password", "пароль", "token", "секрет", "secret", "auth"];

/// Аргументы вызова в одну строку, с вычеркнутыми секретами.
///
/// Разобрать не вышло — пишется отметка, а не сырьё: в сыром виде мог бы уехать пароль.
fn mask_arguments(args: &Value) -> String {
    let Value::Object(map) = args else {
        return "{}".into();
    };
    if map.is_empty() {
        return "{}".into();
    }
    let mut out = serde_json::Map::new();
    for (k, v) in map {
        let lower = k.to_lowercase();
        if SECRET_KEYS.iter().any(|s| lower.contains(s)) {
            out.insert(k.clone(), Value::String("…".into()));
            continue;
        }
        match v {
            Value::String(s) if s.chars().count() > JOURNAL_LIMIT => {
                out.insert(k.clone(), Value::String(shorten(s)));
            }
            _ => {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    serde_json::to_string(&Value::Object(out)).unwrap_or_else(|_| "{нечитаемые аргументы}".into())
}

/// Сводит текст к одной строке ограниченной длины: журнал читают глазами.
fn shorten(s: &str) -> String {
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > JOURNAL_LIMIT {
        let head: String = s.chars().take(JOURNAL_LIMIT).collect();
        return format!("{head}…");
    }
    s
}

/// Длительность в том же виде, что писала Go-версия: миллисекунды.
fn format_duration(d: std::time::Duration) -> String {
    let ms = d.as_millis();
    if ms >= 1000 {
        format!("{:.3}s", d.as_secs_f64())
    } else {
        format!("{ms}ms")
    }
}

include!("tools_table.rs");

#[cfg(test)]
mod tests {
    use super::*;

    fn сервер() -> Server {
        let dir = std::env::temp_dir().join(format!("gfdata-srv-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("bases.json");
        let _ = std::fs::remove_file(&path);
        Server::new(Options {
            registry_path: Some(path),
            ..Default::default()
        })
    }

    fn вызвать(s: &mut Server, запрос: &str) -> Value {
        s.handle(запрос).expect("ответ обязан быть")
    }

    // ── Полнота набора инструментов ───────────────────────────────────────────
    //
    // Ровно здесь и ловится беда, которая уже случилась: при переводе слоя 8 из
    // data.go взяли четыре инструмента из пяти, и eventlog потерялся молча — он
    // не упоминался ни в одном тесте, поэтому ничего не покраснело. Нашёлся
    // случайно, пересчётом структур. Теперь список один и проверяется числом.

    #[test]
    fn инструментов_ровно_шестнадцать() {
        assert_eq!(
            TOOLS.len(),
            16,
            "набор инструментов изменился. Если инструмент добавлен намеренно — поправьте \
             число здесь и убедитесь, что он есть в dispatch; если пропал — это та самая \
             потеря, ради которой тест и стоит"
        );
    }

    #[test]
    fn каждый_объявленный_инструмент_вызывается() {
        // Объявить инструмент и забыть про него в диспетчере — значит показать агенту
        // то, чего нет: он увидит имя в tools/list и получит «нет инструмента».
        let mut s = сервер();
        for t in TOOLS {
            let ответ = вызвать(
                &mut s,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{}","arguments":{{}}}}}}"#,
                    t.name
                ),
            );
            let текст = ответ.to_string();
            assert!(
                !текст.contains("нет инструмента"),
                "{} объявлен, но не разбирается диспетчером",
                t.name
            );
        }
    }

    #[test]
    fn имена_инструментов_не_повторяются() {
        let mut seen = std::collections::HashSet::new();
        for t in TOOLS {
            assert!(seen.insert(t.name), "имя {} объявлено дважды", t.name);
        }
    }

    #[test]
    fn у_каждого_инструмента_есть_описание_и_схема() {
        for t in TOOLS {
            assert!(
                !t.description.trim().is_empty(),
                "{}: пустое описание",
                t.name
            );
            let схема: Value = serde_json::from_str(t.schema)
                .unwrap_or_else(|e| panic!("{}: схема не разобрана: {e}", t.name));
            assert_eq!(схема["type"], "object", "{}: схема не объект", t.name);
        }
    }

    #[test]
    fn база_обязательна_в_схемах_инструментов_данных() {
        // Контракт, который клиент видит ДО вызова: слабая модель не отправляет запрос,
        // который заведомо откажут. Исключение одно — probe, где пустой base значит
        // «проверить все базы».
        for t in TOOLS {
            if matches!(t.name, "probe" | "bases" | "syntax") {
                continue;
            }
            let схема: Value = serde_json::from_str(t.schema).expect("схема");
            let required = схема["required"].as_array().cloned().unwrap_or_default();
            assert!(
                required.iter().any(|v| v == "base"),
                "{}: base не объявлен обязательным",
                t.name
            );
        }
    }

    // ── Протокол ──────────────────────────────────────────────────────────────

    #[test]
    fn initialize_отвечает_именем_и_версией() {
        let mut s = сервер();
        let r = вызвать(
            &mut s,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        );
        assert_eq!(r["result"]["serverInfo"]["name"], SERVER_NAME);
        assert_eq!(r["result"]["serverInfo"]["version"], VERSION);
        assert_eq!(
            r["result"]["protocolVersion"], "2025-06-18",
            "версия клиента знакома — отвечаем ею же"
        );
    }

    #[test]
    fn tools_list_отдаёт_все_инструменты_со_схемами() {
        let mut s = сервер();
        let r = вызвать(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
        let список = r["result"]["tools"].as_array().expect("массив");
        assert_eq!(список.len(), 16);
        for t in список {
            assert!(t["name"].is_string());
            assert!(t["inputSchema"]["type"] == "object", "{}", t["name"]);
        }
    }

    #[test]
    fn схема_построителя_принимает_лояльные_формы() {
        // В таблице у query_build схема, выведенная из полей, — она знает только
        // строгий массив. Подменяться должна рукописная: именно её лояльность
        // (строка вместо списка, массив в строке) уводит модель от ручного сочинения
        // текста запроса.
        let mut s = сервер();
        let r = вызвать(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
        let список = r["result"]["tools"].as_array().expect("массив");
        let qb = список
            .iter()
            .find(|t| t["name"] == "query_build")
            .expect("query_build в списке");
        let where_ = &qb["inputSchema"]["properties"]["where"];
        let варианты = where_["anyOf"].as_array().expect("where должен быть anyOf");
        assert!(
            варианты.iter().any(|v| v["type"] == "string"),
            "схема построителя не принимает условие строкой: {where_}"
        );
    }

    #[test]
    fn уведомление_остаётся_без_ответа() {
        let mut s = сервер();
        assert!(
            s.handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .is_none(),
            "на уведомление отвечать нельзя"
        );
    }

    #[test]
    fn ping_отвечает_пустым_результатом() {
        let mut s = сервер();
        let r = вызвать(&mut s, r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#);
        assert_eq!(r["id"], 7);
        assert_eq!(r["result"], json!({}));
    }

    #[test]
    fn неизвестный_метод_даёт_ошибку_протокола() {
        let mut s = сервер();
        let r = вызвать(
            &mut s,
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/list"}"#,
        );
        assert_eq!(r["error"]["code"], -32601);
    }

    #[test]
    fn мусор_вместо_json_даёт_parse_error() {
        let mut s = сервер();
        let r = s.handle("не json вовсе").expect("ответ обязан быть");
        assert_eq!(r["error"]["code"], -32700);
    }

    #[test]
    fn отказ_инструмента_не_ошибка_протокола() {
        // Ключевое различие: «база не названа» — это успешный ответ JSON-RPC
        // с isError, а не поломка сервера. Смешать их значит лишить клиента
        // возможности отличить одно от другого.
        let mut s = сервер();
        let r = вызвать(
            &mut s,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"metadata","arguments":{}}}"#,
        );
        assert!(r.get("error").is_none(), "это не ошибка протокола: {r}");
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .contains("ОТКАЗ"));
    }

    #[test]
    fn несуществующий_инструмент_это_ошибка_протокола() {
        let mut s = сервер();
        let r = вызвать(
            &mut s,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"нетакого","arguments":{}}}"#,
        );
        assert_eq!(r["error"]["code"], -32602);
    }

    // ── Журнал ────────────────────────────────────────────────────────────────

    #[test]
    fn секреты_в_журнал_не_попадают() {
        let args = json!({
            "name": "buh",
            "password": "тайна",
            "Пароль": "тоже тайна",
            "auth": "ntlm",
            "url": "http://localhost/buh"
        });
        let s = mask_arguments(&args);
        assert!(!s.contains("тайна"), "пароль уехал в журнал: {s}");
        assert!(
            !s.contains("ntlm"),
            "способ аутентификации тоже секрет: {s}"
        );
        assert!(s.contains("buh"), "несекретное обязано остаться: {s}");
        assert!(s.contains("localhost"), "адрес не секрет: {s}");
    }

    #[test]
    fn длинные_значения_обрезаются() {
        let длинный = "я".repeat(500);
        let s = mask_arguments(&json!({ "query": длинный }));
        assert!(
            s.chars().count() < 300,
            "значение не обрезано: {} симв.",
            s.chars().count()
        );
        assert!(s.contains('…'));
    }

    #[test]
    fn итог_различает_отказ_и_поломку() {
        assert!(outcome(&Ok("Строк 5".into())).starts_with("ок"));
        assert!(outcome(&Err(ToolError::Unknown("х".into()))).starts_with("СБОЙ"));
        let r = Refusal::new(
            crate::refusal::Kind::BadRequest,
            "база не названа",
            "нужен base",
        );
        assert!(outcome(&Err(ToolError::Refused(r))).starts_with("ОТКАЗ"));
    }

    #[test]
    fn итог_сводится_к_одной_строке() {
        let многострочный = "первая\nвторая\n\nтретья";
        assert_eq!(shorten(многострочный), "первая вторая третья");
    }

    // ── Инструкции ────────────────────────────────────────────────────────────

    #[test]
    fn инструкции_говорят_главное() {
        // Этот текст влияет на поведение агента сильнее описаний инструментов:
        // если из него уйдёт хоть один из этих пунктов, агент начнёт читать отказ
        // как отсутствие данных.
        for обязательное in [
            "Читай ответы буквально",
            "base ОБЯЗАТЕЛЕН",
            "ОТКАЗ",
            "probe",
            "query_build",
        ] {
            assert!(
                INSTRUCTIONS.contains(обязательное),
                "из инструкций пропало: {обязательное}"
            );
        }
    }
}
