//! Запрос к каталогу LDAP: подключиться, представиться, поискать.
//!
//! Отвечает на два вопроса, которые в организации задают каталогу чаще всего: «пускает ли
//! он с этими учётными данными» и «есть ли там такая запись». Оба возникают при разборе
//! неполадок со входом, и оба обычно решаются командной строкой, которой под рукой нет.
//!
//! Клиент здесь готовый (`ldap3`), а не свой: в отличие от MySQL, где ни одна библиотека
//! не принимала открытый поток, тут ограничение то же, но цена другая. LDAP - это ASN.1,
//! и писать его разбор ради варианта «с сервера» несоразмерно пользе. Поэтому запрос идёт
//! **с машины пользователя**, и в интерфейсе об этом сказано прямо, а не умолчано.

use ldap3::{LdapConnAsync, Scope, SearchEntry};
use serde_json::{json, Value};

/// Сколько записей отдаём за раз.
///
/// Каталог организации - это тысячи записей, и поиск по `(objectClass=*)` вернёт их все.
/// Панели столько не нужно: утилита отвечает «есть или нет и что внутри», а не выгружает
/// справочник.
pub const MAX_ENTRIES: usize = 50;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Params {
    /// `ldap://host` или `ldaps://host:636`.
    pub url: String,
    /// С чем представляемся. Пусто - анонимно.
    #[serde(default)]
    pub bind_dn: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    /// Откуда искать: `dc=example,dc=com`.
    #[serde(default)]
    pub base: Option<String>,
    /// Условие поиска. Пусто - все записи ветки.
    #[serde(default)]
    pub filter: Option<String>,
}

/// Проверка адреса до подключения.
///
/// Схема только `ldap` и `ldaps`: всё остальное здесь означает опечатку, а не намерение,
/// и внятный отказ полезнее, чем ошибка из глубины библиотеки.
pub fn check_url(url: &str) -> Result<(), String> {
    let u = url.trim();
    if u.is_empty() {
        return Err("Пустой адрес каталога".into());
    }
    if !(u.starts_with("ldap://") || u.starts_with("ldaps://")) {
        return Err("Адрес должен начинаться с ldap:// или ldaps://".into());
    }
    Ok(())
}

pub async fn search(p: Params) -> Result<Value, String> {
    check_url(&p.url)?;
    let base = p.base.as_deref().unwrap_or("").trim().to_string();
    let filter = {
        let f = p.filter.as_deref().unwrap_or("").trim();
        if f.is_empty() { "(objectClass=*)".to_string() } else { f.to_string() }
    };

    let started = std::time::Instant::now();
    let (conn, mut ldap) = LdapConnAsync::new(p.url.trim())
        .await
        .map_err(|e| format!("Не удалось соединиться с каталогом: {e}"))?;
    // Соединение - отдельная задача, качающая байты. Без неё запросы не поедут.
    ldap3::drive!(conn);

    match (p.bind_dn.as_deref().filter(|s| !s.trim().is_empty()), p.password.as_deref()) {
        (Some(dn), pass) => {
            ldap.simple_bind(dn, pass.unwrap_or(""))
                .await
                .map_err(|e| format!("Каталог не ответил на попытку входа: {e}"))?
                .success()
                .map_err(|e| format!("Каталог не пустил: {e}"))?;
        }
        // Анонимный вход - законный сценарий: часть каталогов отдаёт публичную ветку без
        // учётных данных, и проверять доступность удобнее именно так.
        (None, _) => {}
    }

    let (rs, _res) = ldap
        .search(&base, Scope::Subtree, &filter, Vec::<String>::new())
        .await
        .map_err(|e| format!("Поиск не выполнился: {e}"))?
        .success()
        .map_err(|e| format!("Каталог отказал в поиске: {e}"))?;

    let всего = rs.len();
    let mut entries: Vec<Value> = Vec::new();
    for row in rs.into_iter().take(MAX_ENTRIES) {
        let e = SearchEntry::construct(row);
        let mut attrs: Vec<Value> = e
            .attrs
            .into_iter()
            .map(|(name, values)| json!({ "name": name, "values": values }))
            .collect();
        // Двоичные значения (фотографии, сертификаты) приходят отдельно. Показывать их
        // содержимое бессмысленно, а вот знать, что они есть, - полезно.
        for (name, values) in e.bin_attrs {
            attrs.push(json!({
                "name": name,
                "values": values.iter().map(|v| format!("<двоичные данные, {} байт>", v.len())).collect::<Vec<_>>(),
            }));
        }
        attrs.sort_by(|a, b| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")));
        entries.push(json!({ "dn": e.dn, "attrs": attrs }));
    }

    let _ = ldap.unbind().await;

    let mut out = json!({
        "url": p.url.trim(),
        "base": base,
        "filter": filter,
        "found": всего,
        "entries": entries,
        "ms": started.elapsed().as_millis(),
    });
    // Про обрезку говорим прямо: «показано 50» без пояснения читается как «всего 50».
    if всего > MAX_ENTRIES {
        out["truncated"] = json!(format!(
            "Показаны первые {MAX_ENTRIES} из {всего} - уточните условие поиска"
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn чужая_схема_отвергается_до_подключения() {
        // Внятный отказ полезнее, чем ошибка из глубины библиотеки про неизвестный порт.
        assert!(check_url("ldap://dc.example.com").is_ok());
        assert!(check_url("ldaps://dc.example.com:636").is_ok());
        assert!(check_url("http://dc.example.com").is_err());
        assert!(check_url("dc.example.com").is_err());
        assert!(check_url("").is_err());
    }

    #[test]
    fn отказ_объясняет_что_именно_не_так() {
        let err = check_url("http://x").unwrap_err();
        assert!(err.contains("ldap://"), "в отказе нет подсказки: {err}");
    }
}
