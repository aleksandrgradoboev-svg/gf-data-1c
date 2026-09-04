//! Канал к HTTP-сервису расширения внутри информационной базы.
//!
//! Здесь живёт главное свойство продукта: неуспех обращения превращается в отказ
//! НУЖНОГО вида, а не в пустой ответ. Погашенный веб-сервер, неопубликованная база,
//! отсутствующее расширение и отказ прав — четыре разные беды с четырьмя разными
//! лечениями, и различать их приходится по косвенным признакам, потому что коды
//! ответа у части из них совпадают.

use std::time::{Duration, Instant};

use crate::journal;
use crate::ntlm;
use crate::refusal::{Kind, Refusal};
use crate::registry::Base;

/// Сколько ждать базу. Триста секунд не роскошь: выгрузка метаданных крупной
/// конфигурации идёт минутами, и таймаут короче превращает медленный ответ в ложный
/// «канал мёртв».
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// Потолок ответа базы, 128 МиБ.
///
/// Нужен не ради памяти, а ради внятности: ответ на порядок больше ожидаемого — это
/// почти всегда ошибка запроса, и упереться в понятный отказ лучше, чем в загадочный
/// сбой разбора где-то в середине гигабайта.
pub const MAX_RESPONSE_BYTES: usize = 128 << 20;

/// Канал к одной базе.
pub struct Client {
    base: Base,
    http: reqwest::blocking::Client,
}

impl Client {
    /// Создаёт канал.
    ///
    /// Таймаут задан явно: агент, ждущий базу бесконечно, выглядит зависшим, а не
    /// отказавшим.
    ///
    /// Пул соединений включён и для NTLM обязателен: сервер помнит выданный challenge
    /// на TCP-соединении, и переоткрытое между шагами рукопожатия соединение обнуляет
    /// обмен — правильный ответ получает 401.
    pub fn new(base: Base, timeout: Option<Duration>) -> Result<Self, Refusal> {
        let timeout = timeout.filter(|t| !t.is_zero()).unwrap_or(DEFAULT_TIMEOUT);
        let http = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .pool_max_idle_per_host(4)
            .build()
            .map_err(|e| Refusal::new(Kind::Internal, "HTTP-клиент не создан", e.to_string()))?;
        Ok(Self { base, http })
    }

    /// База, к которой подключён канал.
    pub fn base(&self) -> &Base {
        &self.base
    }

    /// GET к методу расширения.
    pub fn get(&self, method: &str, query: &[(&str, &str)]) -> Result<Vec<u8>, Refusal> {
        self.do_request(reqwest::Method::GET, method, query, None)
    }

    /// POST с телом JSON — для запросов, которые не помещаются в URL.
    pub fn post_json<T: serde::Serialize>(
        &self,
        method: &str,
        payload: &T,
    ) -> Result<Vec<u8>, Refusal> {
        let body = serde_json::to_vec(payload)
            .map_err(|e| Refusal::new(Kind::Internal, "запрос не сериализован", e.to_string()))?;
        self.do_request(reqwest::Method::POST, method, &[], Some(body))
    }

    /// Единственная точка выхода обращений к базе, и потому единственное место, где
    /// отказу проставляется её имя. Раскладывать `stamp` по всем ветвям было бы то же
    /// самое, что просить не забывать: следующая добавленная ветвь про это не узнает.
    fn do_request(
        &self,
        verb: reqwest::Method,
        method: &str,
        query: &[(&str, &str)],
        body: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, Refusal> {
        self.do_raw(verb, method, query, body)
            .map_err(|e| e.stamp(&self.base.name))
    }

    fn do_raw(
        &self,
        verb: reqwest::Method,
        method: &str,
        query: &[(&str, &str)],
        body: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, Refusal> {
        let endpoint = format!(
            "{}/{}",
            self.base.url.trim_end_matches('/'),
            method.trim_start_matches('/')
        );

        let started = Instant::now();
        let response = self.send(&verb, &endpoint, query, body.as_deref())?;
        let status = response.status();

        journal::writef(format_args!(
            "{} {} {} → {} за {}ms",
            self.base.name,
            verb,
            endpoint,
            status.as_u16(),
            started.elapsed().as_millis()
        ));

        // Читаем на байт больше потолка: превышение видно сразу, а не после того,
        // как ответ уже съел память.
        let data = response
            .bytes()
            .map_err(|e| Refusal::new(Kind::BaseError, "ответ базы не прочитан", e.to_string()))?;
        if data.len() > MAX_RESPONSE_BYTES {
            return Err(Refusal::new(
                Kind::BaseError,
                "ответ базы слишком велик",
                format!("превышен потолок {} МиБ", MAX_RESPONSE_BYTES >> 20),
            )
            .hint(
                "сузьте запрос: перечислите нужные поля вместо звёздочки, поставьте limit, \
                 добавьте отбор по периоду",
            ));
        }

        self.classify_status(status.as_u16(), &data)?;
        Ok(data.to_vec())
    }

    /// Отправляет запрос, проводя при необходимости доменное рукопожатие.
    fn send(
        &self,
        verb: &reqwest::Method,
        endpoint: &str,
        query: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<reqwest::blocking::Response, Refusal> {
        let ntlm_needed = ntlm::needed(&self.base.auth, &self.base.user);

        if !ntlm_needed {
            let req = self.build(verb, endpoint, query, body)?;
            let req = match (&self.base.user, self.password()?) {
                (u, p) if !u.is_empty() => req.basic_auth(u, Some(p)),
                _ => req,
            };
            return req
                .send()
                .map_err(|e| self.classify_transport(&e, endpoint));
        }

        // ── Доменное рукопожатие: три шага по одному соединению ────────────
        //
        // Первый запрос заведомо получит 401 — он и нужен ради challenge в заголовке.
        let negotiate = ntlm::negotiate_header()?;
        let first = self
            .build(verb, endpoint, query, body)?
            .header(reqwest::header::AUTHORIZATION, negotiate)
            .send()
            .map_err(|e| self.classify_transport(&e, endpoint))?;

        if first.status() != reqwest::StatusCode::UNAUTHORIZED {
            // Сервер не стал спрашивать — значит доступ уже есть либо NTLM не нужен.
            return Ok(first);
        }

        let challenge = first
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();

        if !challenge.to_ascii_uppercase().contains("NTLM") {
            // Сервер отказал, но доменного обмена не предложил: это обычный отказ
            // прав, и выдавать его за неудачу рукопожатия нельзя — лечится он иначе.
            return Ok(first);
        }

        let creds = ntlm::Credentials::parse(&self.base.user, &self.password()?);
        let authenticate = ntlm::authenticate_header(&challenge, &creds)?;

        self.build(verb, endpoint, query, body)?
            .header(reqwest::header::AUTHORIZATION, authenticate)
            .send()
            .map_err(|e| self.classify_transport(&e, endpoint))
    }

    fn build(
        &self,
        verb: &reqwest::Method,
        endpoint: &str,
        query: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<reqwest::blocking::RequestBuilder, Refusal> {
        let mut req = self
            .http
            .request(verb.clone(), endpoint)
            .header(reqwest::header::ACCEPT, "application/json");
        if !query.is_empty() {
            req = req.query(query);
        }
        if let Some(b) = body {
            req = req
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/json; charset=utf-8",
                )
                .body(b.to_vec());
        }
        Ok(req)
    }

    /// Пароль в открытом виде. Учётные данные живут отдельно от адреса и в URL не
    /// попадают никогда — иначе рано или поздно уедут в журнал вместе с адресом.
    fn password(&self) -> Result<String, Refusal> {
        self.base.reveal_password().map_err(|_| {
            Refusal::new(
                Kind::BadRequest,
                "пароль базы не прочитан",
                "защищённое значение не расшифровано",
            )
            .hint("перезапишите учётные данные: bases с action=add")
        })
    }

    /// Различает «сервера нет» и прочие сетевые беды.
    fn classify_transport(&self, err: &reqwest::Error, endpoint: &str) -> Refusal {
        // В журнал идёт адрес без учётных данных: они живут отдельно от URL и в текст
        // запроса не подставляются.
        journal::writef(format_args!(
            "{} {} → не отвечает: {}",
            self.base.name, endpoint, err
        ));

        if err.is_timeout() {
            return Refusal::new(
                Kind::NoWebServer,
                "база не ответила вовремя",
                "истёк таймаут",
            )
            .hint("веб-сервер может быть занят перепроведением или выгрузкой")
            .hint("проверьте канал инструментом probe");
        }
        if err.is_connect() {
            return Refusal::new(
                Kind::NoWebServer,
                "соединение с базой не установлено",
                "адрес не принимает подключение",
            )
            .hint(
                "веб-сервер публикации 1С не поднят — он запускается процессом и после \
                 перезагрузки машины исчезает молча",
            )
            .hint("проверьте канал инструментом probe");
        }
        Refusal::new(
            Kind::NoWebServer,
            "обращение к базе не удалось",
            err.to_string(),
        )
        .hint("проверьте канал инструментом probe")
    }

    /// Превращает код ответа в отказ нужного вида.
    fn classify_status(&self, code: u16, body: &[u8]) -> Result<(), Refusal> {
        match code {
            200..=299 => Ok(()),
            404 => {
                // Два РАЗНЫХ 404, и лечатся они по-разному. Различает их не длина тела,
                // а его происхождение: HTML-страницу «404 Not Found» рисует сам веб-сервер,
                // когда до 1С запрос не дошёл (нет такой публикации), а 1С на неизвестный
                // маршрут отвечает пустым телом без Content-Type. Замерено на живом стенде
                // 03.09.2026; прежде оба случая назывались «нет расширения», и человек искал
                // расширение там, где не было опубликовано базы.
                let html = contains_ignore_case(body, b"<html");
                if html {
                    Err(Refusal::new(
                        Kind::NoPublication,
                        "базы нет по этому адресу",
                        "HTTP 404 страницей веб-сервера — до 1С запрос не дошёл",
                    )
                    .hint(format!("проверьте имя публикации в адресе: {}", self.base.url))
                    .hint(
                        "либо база не опубликована на веб-сервере (публикация — отдельное действие)",
                    ))
                } else {
                    Err(Refusal::new(
                        Kind::NoExtension,
                        "расширение не отвечает в этой базе",
                        "HTTP 404 от самой 1С — база отвечает, маршрута нет",
                    )
                    .hint(format!(
                        "расширение доступа к данным не установлено в базе {} или выключено",
                        self.base.name
                    ))
                    .hint(
                        "проверьте: Конфигурация → Расширения — расширение GTData должно быть активно",
                    )
                    .hint(
                        "либо публикация выполнена без HTTP-сервисов (отдельная галка в Конфигураторе)",
                    ))
                }
            }
            401 | 403 => {
                // Две разные причины дают один и тот же 401, и лечатся они по-разному,
                // поэтому названы обе. Вторая — типичная для расширения, поставленного
                // РУКАМИ через Конфигуратор: установщик объявляет роль расширения основной
                // сам, а при ручной загрузке cfe роль остаётся невыданной.
                //
                // Причин у 401 три, и платформа их НЕ различает: «нет такого пользователя»,
                // «неверный пароль» и «нет права на сервис» дают байт-в-байт одинаковый
                // ответ (401, WWW-Authenticate: Basic, пустое тело) — замерено 03.09.2026.
                // Это верное поведение 1С: иначе перебор логинов стал бы разведкой.
                // Поэтому называются все три, а не выбирается одна наугад.
                Err(Refusal::new(
                    Kind::Unauthorized,
                    "база отказала в доступе",
                    format!("HTTP {code} — какая из причин, платформа не сообщает"),
                )
                .hint(format!(
                    "либо в базе нет пользователя {}, либо неверен пароль: поправить — \
                     bases с action=update (пустой пароль не трогается)",
                    self.base.user
                ))
                .hint(
                    "либо пользователю не выдано право на сервис: роль GT_ОсновнаяРоль выдаётся \
                     в пользовательском режиме через профиль группы доступа (Администрирование → \
                     Настройки пользователей и прав → Профили групп доступа), а не в конфигураторе",
                ))
            }
            _ => Err(Refusal::new(
                Kind::BaseError,
                "база ответила ошибкой",
                format!("HTTP {code}: {}", snippet(body)),
            )),
        }
    }
}

/// Поиск подстроки без учёта регистра — тело 404 приходит и как `<html`, и как `<HTML`.
fn contains_ignore_case(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|w| w.iter().zip(needle).all(|(a, b)| a.eq_ignore_ascii_case(b)))
}

/// Кусок тела ответа для сообщения об ошибке.
fn snippet(body: &[u8]) -> String {
    let s = String::from_utf8_lossy(body);
    let s = s.trim();
    if s.chars().count() > 300 {
        let cut: String = s.chars().take(300).collect();
        format!("{cut}…")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn канал(base: Base) -> Client {
        Client::new(base, Some(Duration::from_secs(5))).unwrap()
    }

    fn база() -> Base {
        Base {
            name: "ut11".into(),
            url: "http://localhost:8081/ut11/hs/gt-data".into(),
            user: "agent".into(),
            ..Default::default()
        }
    }

    #[test]
    fn успех_не_даёт_отказа() {
        assert!(канал(база()).classify_status(200, b"{}").is_ok());
        assert!(канал(база()).classify_status(204, b"").is_ok());
    }

    #[test]
    fn два_разных_404_различаются() {
        let c = канал(база());

        let веб = c
            .classify_status(404, b"<html><body>404 Not Found</body></html>")
            .unwrap_err();
        assert_eq!(веб.kind, Kind::NoPublication, "страницу рисует веб-сервер");

        let расширение = c.classify_status(404, b"").unwrap_err();
        assert_eq!(
            расширение.kind,
            Kind::NoExtension,
            "пустое тело — ответ самой 1С"
        );
    }

    #[test]
    fn html_в_верхнем_регистре_тоже_опознаётся() {
        let err = канал(база())
            .classify_status(404, b"<HTML><BODY>404</BODY></HTML>")
            .unwrap_err();
        assert_eq!(err.kind, Kind::NoPublication);
    }

    #[test]
    fn отказ_прав_называет_все_три_причины() {
        let err = канал(база()).classify_status(401, b"").unwrap_err();
        assert_eq!(err.kind, Kind::Unauthorized);
        let text = err.to_string();
        assert!(text.contains("нет пользователя"), "{text}");
        assert!(text.contains("неверен пароль"), "{text}");
        assert!(text.contains("право на сервис"), "{text}");
        assert!(
            text.contains("платформа не сообщает"),
            "выбирать одну причину наугад нельзя: {text}"
        );
    }

    #[test]
    fn отказ_называет_базу() {
        let c = канал(база());
        let err = c.classify_status(404, b"").unwrap_err().stamp(&c.base.name);
        assert!(err.to_string().starts_with("ОТКАЗ (база ut11)"), "{err}");
    }

    #[test]
    fn прочие_коды_показывают_кусок_ответа() {
        let err = канал(база())
            .classify_status(500, "внутренняя ошибка базы".as_bytes())
            .unwrap_err();
        assert_eq!(err.kind, Kind::BaseError);
        assert!(err.to_string().contains("внутренняя ошибка базы"), "{err}");
    }

    #[test]
    fn длинное_тело_обрезается() {
        let длинное = "я".repeat(1000);
        let s = snippet(длинное.as_bytes());
        assert!(s.ends_with('…'));
        assert_eq!(s.chars().count(), 301, "300 знаков плюс многоточие");
    }
}
