//! Решение «нужен ли NTLM» — сверка с Go-версией. Ошибка здесь даёт отказ прав,
//! который читается как «не тот пароль».
fn main() {
    let cases = [
        ("", r"ДОМЕН\вася"),
        ("", "вася"),
        ("ntlm", "вася"),
        ("basic", r"ДОМЕН\вася"),
        ("NTLM", "вася"),
        ("Negotiate", "вася"),
        ("kerberos", "вася"),
        ("DOMAIN", "вася"),
        ("  ntlm  ", "вася"),
        ("чепуха", r"ДОМЕН\вася"),
        ("чепуха", "вася"),
    ];
    for (auth, user) in cases {
        println!(
            "auth={auth:?} user={user:?} → ntlm={}",
            gf_data_1c::ntlm::needed(auth, user)
        );
    }
}
