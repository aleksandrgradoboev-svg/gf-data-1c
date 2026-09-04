//! Защита паролей реестра баз средствами операционной системы.
//!
//! Смысл не в стойкости шифра, а в области действия ключа: DPAPI привязывает данные
//! к учётной записи пользователя, поэтому украденный файл реестра ничего не даёт
//! на другой машине и под другим пользователем. Открытый пароль в файле давал.
//!
//! Формат хранения — `dpapi:<base64>` — тот же, что писала версия на Go. Это не
//! косметика: реестр переживает смену реализации, и уже защищённые пароли обязаны
//! читаться дальше. Проверяется тестом на выводе Go-версии.

/// Метка защищённого значения. По ней отличается уже зашифрованный пароль от того,
/// что пользователь вписал в реестр руками.
pub const PREFIX: &str = "dpapi:";

/// Защищено ли значение. Проверка по метке, без обращения к криптографии.
pub fn is_protected(value: &str) -> bool {
    value.starts_with(PREFIX)
}

#[cfg(windows)]
mod imp {
    use super::PREFIX;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
    };

    /// Освобождает память, выданную DPAPI. Забыть об этом нельзя: буфер выделяет
    /// система, и владелец у него один.
    struct Owned(CRYPT_INTEGER_BLOB);

    impl Owned {
        fn bytes(&self) -> Vec<u8> {
            if self.0.pbData.is_null() || self.0.cbData == 0 {
                return Vec::new();
            }
            // SAFETY: указатель и длина получены от самой DPAPI одним вызовом,
            // память жива до LocalFree в Drop.
            unsafe { std::slice::from_raw_parts(self.0.pbData, self.0.cbData as usize) }.to_vec()
        }
    }

    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.pbData.is_null() {
                // SAFETY: буфер выдан DPAPI и ещё не освобождался.
                unsafe {
                    let _ = LocalFree(Some(HLOCAL(self.0.pbData as *mut _)));
                }
            }
        }
    }

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    /// Шифрует значение под текущего пользователя.
    ///
    /// Уже защищённое возвращается как есть: повторное шифрование сделало бы файл
    /// нечитаемым для самого себя после второго сохранения.
    pub fn protect(value: &str) -> Result<String, String> {
        if value.is_empty() || super::is_protected(value) {
            return Ok(value.to_string());
        }

        let input = value.as_bytes();
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: обе структуры живут до конца вызова; флаги нулевые — интерфейс
        // не запрашивается, сервер работает без пользователя за экраном.
        let ok = unsafe {
            CryptProtectData(
                &blob(input),
                None,
                None,
                None,
                None,
                Default::default(),
                &mut out,
            )
        };
        match ok {
            Ok(()) => {
                let owned = Owned(out);
                Ok(format!("{PREFIX}{}", STANDARD.encode(owned.bytes())))
            }
            Err(e) => Err(format!("пароль не зашифрован: {e}")),
        }
    }

    /// Расшифровывает значение. Незащищённое возвращается как есть — реестр
    /// разрешается править руками, и вписанный открытый пароль обязан работать.
    pub fn reveal(value: &str) -> Result<String, String> {
        if !super::is_protected(value) {
            return Ok(value.to_string());
        }

        let raw = STANDARD
            .decode(value.trim_start_matches(PREFIX))
            .map_err(|e| format!("защищённый пароль испорчен: {e}"))?;

        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: см. protect.
        let ok = unsafe {
            CryptUnprotectData(
                &blob(&raw),
                None,
                None,
                None,
                None,
                Default::default(),
                &mut out,
            )
        };
        match ok {
            Ok(()) => {
                let owned = Owned(out);
                String::from_utf8(owned.bytes())
                    .map_err(|e| format!("расшифрованный пароль не текст: {e}"))
            }
            Err(e) => Err(format!("пароль не расшифрован: {e}")),
        }
    }

    /// Есть ли защита на этой платформе.
    pub fn available() -> bool {
        true
    }
}

#[cfg(not(windows))]
mod imp {
    //! Заглушка: DPAPI есть только в Windows.
    //!
    //! Молча «шифровать» ничем нельзя — это создало бы видимость защиты. Поэтому
    //! значение возвращается как есть, а `available()` честно отвечает, что защиты
    //! нет: вызывающий решает сам, предупреждать пользователя или отказываться.

    pub fn protect(value: &str) -> Result<String, String> {
        Ok(value.to_string())
    }

    pub fn reveal(value: &str) -> Result<String, String> {
        if super::is_protected(value) {
            return Err("защищённый пароль прочитать нечем: DPAPI есть только в Windows".into());
        }
        Ok(value.to_string())
    }

    pub fn available() -> bool {
        false
    }
}

pub use imp::{available, protect, reveal};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn метка_отличает_защищённое_от_вписанного_руками() {
        assert!(is_protected("dpapi:AQAAA"));
        assert!(!is_protected("обычный-пароль"));
        assert!(!is_protected(""));
    }

    #[test]
    fn незащищённое_читается_как_есть() {
        assert_eq!(reveal("пароль123").unwrap(), "пароль123");
    }

    #[test]
    fn пустое_значение_не_шифруется() {
        assert_eq!(protect("").unwrap(), "");
    }

    #[cfg(windows)]
    #[test]
    fn защита_обратима() {
        let исходный = "пароль с пробелами и кириллицей";
        let закрытый = protect(исходный).unwrap();
        assert!(
            is_protected(&закрытый),
            "результат обязан нести метку, иначе его примут за открытый пароль"
        );
        assert_ne!(закрытый, исходный);
        assert_eq!(reveal(&закрытый).unwrap(), исходный);
    }

    #[cfg(windows)]
    #[test]
    fn повторная_защита_не_шифрует_дважды() {
        let один = protect("пароль").unwrap();
        let два = protect(&один).unwrap();
        assert_eq!(
            один, два,
            "иначе файл станет нечитаемым для себя после второго сохранения"
        );
    }

    #[cfg(windows)]
    #[test]
    fn испорченный_base64_даёт_внятный_отказ() {
        let err = reveal("dpapi:это-не-base64!!!").unwrap_err();
        assert!(err.contains("испорчен"), "{err}");
    }
}
