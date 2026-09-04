//! Доменная аутентификация NTLMv2 поверх HTTP.
//!
//! В версии на Go это была одна строка: обёртка транспорта `ntlmssp.Negotiator` делала
//! рукопожатие сама. В Rust готовой обёртки для клиента нет (разведка 04.09.2026), поэтому
//! трёхшаговый обмен собран здесь, а криптография взята готовой у `ntlmclient`.
//!
//! Порядок обмена (MS-NLMP):
//!
//! 1. запрос с `Authorization: NTLM <negotiate>` — сервер отвечает 401 и своим challenge;
//! 2. из challenge считается ответ NTLMv2 с учётными данными;
//! 3. запрос повторяется с `Authorization: NTLM <authenticate>` — и вот он выполняется.
//!
//! Обмен привязан к TCP-соединению: сервер помнит выданный challenge на соединении, а не
//! в сессии. Поэтому все три шага идут одним `reqwest::Client` с включённым пулом —
//! переоткрытое между шагами соединение обнуляет рукопожатие, и сервер отвечает 401 на
//! правильный ответ. Это главное, чем NTLM отличается от Basic, и главное, что здесь
//! легко сломать незаметно.

use base64::{engine::general_purpose::STANDARD, Engine};

use crate::refusal::{Kind, Refusal};

/// Учётные данные, разобранные на домен и имя.
///
/// Логин приходит в виде `ДОМЕН\пользователь` — так его пишут в реестре баз, и так же
/// по нему опознаётся необходимость NTLM.
pub struct Credentials {
    pub domain: String,
    pub user: String,
    pub password: String,
}

impl Credentials {
    /// Разбирает `ДОМЕН\пользователь`. Логин без косой черты — учётка без домена:
    /// такое NTLM тоже допускает, домен тогда пустой.
    pub fn parse(login: &str, password: &str) -> Self {
        match login.split_once('\\') {
            Some((domain, user)) => Self {
                domain: domain.to_string(),
                user: user.to_string(),
                password: password.to_string(),
            },
            None => Self {
                domain: String::new(),
                user: login.to_string(),
                password: password.to_string(),
            },
        }
    }
}

/// Имя рабочей станции для сообщений NTLM. Сервер его не проверяет, но поле обязательное.
fn workstation() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "workstation".to_string())
}

/// Первое сообщение обмена — «умею NTLM, вот кто я».
pub fn negotiate_header() -> Result<String, Refusal> {
    let flags = ntlmclient::Flags::NEGOTIATE_UNICODE
        | ntlmclient::Flags::REQUEST_TARGET
        | ntlmclient::Flags::NEGOTIATE_NTLM
        | ntlmclient::Flags::NEGOTIATE_WORKSTATION_SUPPLIED;

    let msg = ntlmclient::Message::Negotiate(ntlmclient::NegotiateMessage {
        flags,
        supplied_domain: String::new(),
        supplied_workstation: workstation(),
        os_version: Default::default(),
    });

    let bytes = msg.to_bytes().map_err(|e| {
        Refusal::new(
            Kind::Internal,
            "сообщение NTLM не собрано",
            format!("{e:?}"),
        )
    })?;
    Ok(format!("NTLM {}", STANDARD.encode(bytes)))
}

/// Третье сообщение обмена — ответ на challenge сервера.
///
/// `challenge_header` — значение заголовка `WWW-Authenticate` из ответа 401, вида
/// `NTLM <base64>`.
pub fn authenticate_header(challenge_header: &str, creds: &Credentials) -> Result<String, Refusal> {
    let b64 = challenge_header.split_whitespace().nth(1).ok_or_else(|| {
        Refusal::new(
            Kind::Unauthorized,
            "доменное рукопожатие не состоялось",
            "сервер ответил заголовком NTLM без самого challenge",
        )
        .hint("возможно, на сервере включён Negotiate/Kerberos вместо NTLM")
    })?;

    let raw = STANDARD.decode(b64).map_err(|e| {
        Refusal::new(
            Kind::Unauthorized,
            "доменное рукопожатие не состоялось",
            format!("challenge сервера не разобран: {e}"),
        )
    })?;

    let message = ntlmclient::Message::try_from(raw.as_slice()).map_err(|e| {
        Refusal::new(
            Kind::Unauthorized,
            "доменное рукопожатие не состоялось",
            format!("challenge сервера не разобран: {e:?}"),
        )
    })?;

    let challenge = match message {
        ntlmclient::Message::Challenge(c) => c,
        other => {
            return Err(Refusal::new(
                Kind::Unauthorized,
                "доменное рукопожатие не состоялось",
                format!(
                    "сервер прислал не challenge, а сообщение №{}",
                    other.message_number()
                ),
            ));
        }
    };

    let target_info: Vec<u8> = challenge
        .target_information
        .iter()
        .flat_map(|e| e.to_bytes())
        .collect();

    let creds = ntlmclient::Credentials {
        username: creds.user.clone(),
        password: creds.password.clone(),
        domain: creds.domain.clone(),
    };

    let response = ntlmclient::respond_challenge_ntlm_v2(
        challenge.challenge,
        &target_info,
        ntlmclient::get_ntlm_time(),
        &creds,
    );

    let flags = ntlmclient::Flags::NEGOTIATE_UNICODE | ntlmclient::Flags::NEGOTIATE_NTLM;
    let auth = response.to_message(&creds, &workstation(), flags);
    let bytes = auth.to_bytes().map_err(|e| {
        Refusal::new(
            Kind::Internal,
            "сообщение NTLM не собрано",
            format!("{e:?}"),
        )
    })?;

    Ok(format!("NTLM {}", STANDARD.encode(bytes)))
}

/// Нужен ли доменный способ для этой базы.
///
/// Явное указание сильнее догадки, но и без него учётка с обратной косой чертой
/// опознаётся как доменная: заставлять человека писать `auth=ntlm` там, где это видно
/// по логину, — лишний повод для отказа прав, который читается как «не тот пароль».
pub fn needed(auth: &str, user: &str) -> bool {
    match auth.trim().to_ascii_lowercase().as_str() {
        "ntlm" | "negotiate" | "kerberos" | "domain" => true,
        "basic" => false,
        _ => user.contains('\\'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn доменная_учётка_опознаётся_по_обратной_косой() {
        assert!(needed("", r"ДОМЕН\вася"));
        assert!(!needed("", "вася"));
    }

    #[test]
    fn явное_указание_сильнее_догадки() {
        assert!(needed("ntlm", "вася"), "сказано ntlm — значит ntlm");
        assert!(
            !needed("basic", r"ДОМЕН\вася"),
            "сказано basic — догадка по логину не спорит"
        );
    }

    #[test]
    fn синонимы_доменного_способа_принимаются() {
        for v in ["NTLM", "Negotiate", "kerberos", "DOMAIN"] {
            assert!(needed(v, "вася"), "{v}");
        }
    }

    #[test]
    fn логин_разбирается_на_домен_и_имя() {
        let c = Credentials::parse(r"CORP\ivanov", "пароль");
        assert_eq!(c.domain, "CORP");
        assert_eq!(c.user, "ivanov");
        assert_eq!(c.password, "пароль");
    }

    #[test]
    fn логин_без_домена_допустим() {
        let c = Credentials::parse("ivanov", "пароль");
        assert!(c.domain.is_empty());
        assert_eq!(c.user, "ivanov");
    }

    #[test]
    fn первое_сообщение_собирается_и_помечено_ntlm() {
        let h = negotiate_header().expect("negotiate не собрался");
        assert!(h.starts_with("NTLM "), "{h}");
        let raw = STANDARD.decode(h.trim_start_matches("NTLM ")).unwrap();
        assert_eq!(
            &raw[..8],
            b"NTLMSSP\0",
            "подпись протокола обязана быть первой"
        );
    }

    #[test]
    fn заголовок_без_challenge_даёт_внятный_отказ() {
        let creds = Credentials::parse(r"CORP\ivanov", "пароль");
        let err = authenticate_header("NTLM", &creds).unwrap_err();
        assert_eq!(err.kind, Kind::Unauthorized);
        assert!(err.to_string().contains("без самого challenge"), "{err}");
    }

    #[test]
    fn испорченный_challenge_не_принимается_за_успех() {
        let creds = Credentials::parse(r"CORP\ivanov", "пароль");
        let err = authenticate_header("NTLM это-не-base64!!!", &creds).unwrap_err();
        assert_eq!(err.kind, Kind::Unauthorized);
    }
}
