//! Цепочка подключения: цель и её jump-хосты, с политикой администратора на каждом звене.
//!
//! Через эту функцию идут все SSH-подключения приложения - сессия, рабочий стол, Fleet,
//! задачи. Поэтому проверка политики здесь, а не у каждого вызова: пропустить звено
//! здесь значит пропустить его везде.

use crate::policy::{self, Policy};
use serde_json::Value;
use std::collections::HashSet;

/// Цепочка для сервера из профиля с расшифрованными секретами: `[цель, прыжок 1, прыжок 2, ...]` -
/// в таком порядке её ждёт `ssh::connect_chain`.
pub fn resolve(server_id: &str) -> Result<Vec<Value>, String> {
    resolve_with(server_id, crate::store::server_with_secrets, policy::current())
}

/// То же, но профиль и политика приходят снаружи - так цепочку можно проверить в тестах.
pub fn resolve_with(server_id: &str, lookup: impl Fn(&str) -> Option<Value>, p: &Policy) -> Result<Vec<Value>, String> {
    let mut chain: Vec<Value> = Vec::new();
    let mut seen = HashSet::new();
    let mut id = Some(server_id.to_string());
    while let Some(sid) = id {
        if !seen.insert(sid.clone()) {
            return Err("Циклическая цепочка jump-хостов".into());
        }
        let mut s = lookup(&sid).ok_or("Сервер из цепочки jump-хостов не найден")?;
        // COM-порт по SSH не открывается - ни целью, ни прыжком. Раньше пометка «serial»
        // снимала со звена проверку политики, а прыжок всё равно шёл по SSH к его `host`:
        // так запрещённый адрес проходил jump-хостом.
        if s.get("connection").and_then(|v| v.as_str()) == Some("serial") {
            return Err(if chain.is_empty() {
                format!("{} - COM-порт, по SSH к нему не подключиться", name_of(&s))
            } else {
                format!("{} - COM-порт и не может быть jump-хостом", name_of(&s))
            });
        }
        // Политика проверяет каждое звено, а не только цель: иначе запрещённый адрес прошёл
        // бы jump-хостом.
        policy::check_host_with(p, s.get("host").and_then(|v| v.as_str()).unwrap_or(""))?;
        check_proxy_command(&s, p)?;
        // Сохранённые раньше пароли при запрете не идут в ход: их спросят при подключении.
        if p.forbid_saved_passwords {
            if let Some(o) = s.as_object_mut() {
                o.remove("password");
                o.remove("passphrase");
            }
        }
        let next = s
            .get("proxyJump")
            .and_then(|v| v.as_str())
            .filter(|x| !x.is_empty())
            .map(|x| x.to_string());
        chain.push(s);
        id = next;
    }
    Ok(chain)
}

/// ProxyCommand запускает на этой машине произвольную команду, и соединяется она куда
/// захочет: адрес в `host` при этом ничего не значит. Поэтому при ограничении адресов он
/// обходил бы `allowedHosts`, а при запрете локального терминала был бы локальной оболочкой
/// в обход запрета. Под такой политикой сервер с ProxyCommand не открывается.
fn check_proxy_command(server: &Value, p: &Policy) -> Result<(), String> {
    let has = server
        .get("proxyCommand")
        .and_then(|v| v.as_str())
        .is_some_and(|c| !c.trim().is_empty());
    if !has {
        return Ok(());
    }
    let why = if p.allowed_hosts.is_some() {
        "политика администратора ограничивает адреса, а ProxyCommand соединяется в обход этого списка"
    } else if p.forbid_local_terminal {
        "политика администратора запрещает локальный терминал, а ProxyCommand - это команда на этой машине"
    } else {
        return Ok(());
    };
    Err(format!("{}: ProxyCommand запрещён - {why}", name_of(server)))
}

fn name_of(server: &Value) -> String {
    let pick = |k: &str| server.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty());
    format!("«{}»", pick("name").or_else(|| pick("host")).unwrap_or("сервер"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn profile(servers: &[Value]) -> impl Fn(&str) -> Option<Value> {
        let map: HashMap<String, Value> = servers
            .iter()
            .map(|s| (s["id"].as_str().unwrap().to_owned(), s.clone()))
            .collect();
        move |id| map.get(id).cloned()
    }

    fn allowed(list: &[&str]) -> Policy {
        Policy {
            allowed_hosts: Some(list.iter().map(|s| s.to_string()).collect()),
            ..Default::default()
        }
    }

    fn hosts(chain: &[Value]) -> Vec<&str> {
        chain.iter().map(|s| s["host"].as_str().unwrap()).collect()
    }

    #[test]
    fn цепочка_идёт_от_цели_к_прыжкам() {
        let p = profile(&[
            json!({ "id": "t", "host": "10.0.0.5", "proxyJump": "j1" }),
            json!({ "id": "j1", "host": "10.0.0.2", "proxyJump": "j2" }),
            json!({ "id": "j2", "host": "10.0.0.1" }),
        ]);
        let chain = resolve_with("t", p, &Policy::default()).unwrap();
        assert_eq!(hosts(&chain), ["10.0.0.5", "10.0.0.2", "10.0.0.1"]);
    }

    #[test]
    fn цикл_и_пропавшее_звено_отклоняются() {
        let cycle = profile(&[
            json!({ "id": "a", "host": "a", "proxyJump": "b" }),
            json!({ "id": "b", "host": "b", "proxyJump": "a" }),
        ]);
        assert_eq!(
            resolve_with("a", cycle, &Policy::default()).err().unwrap(),
            "Циклическая цепочка jump-хостов"
        );
        let own = profile(&[json!({ "id": "a", "host": "a", "proxyJump": "a" })]);
        assert!(resolve_with("a", own, &Policy::default()).is_err());
        let gone = profile(&[json!({ "id": "a", "host": "a", "proxyJump": "нет" })]);
        assert!(resolve_with("a", gone, &Policy::default()).is_err());
    }

    #[test]
    fn политика_проверяет_каждое_звено() {
        let p = profile(&[
            json!({ "id": "t", "host": "10.0.0.5", "proxyJump": "j" }),
            json!({ "id": "j", "host": "192.168.1.1" }),
        ]);
        let e = resolve_with("t", p, &allowed(&["10.0.0.0/24"])).err().unwrap();
        assert!(e.contains("192.168.1.1"), "{e}");
    }

    #[test]
    fn пометка_com_порт_не_снимает_проверку_с_прыжка() {
        // Раньше такой прыжок проходил мимо allowedHosts, а подключение к нему шло по SSH.
        let p = profile(&[
            json!({ "id": "t", "host": "10.0.0.5", "proxyJump": "j" }),
            json!({ "id": "j", "name": "обход", "host": "192.168.1.1", "connection": "serial" }),
        ]);
        let e = resolve_with("t", &p, &allowed(&["10.0.0.0/24"])).err().unwrap();
        assert!(e.contains("не может быть jump-хостом"), "{e}");
        assert!(
            resolve_with("t", &p, &Policy::default()).is_err(),
            "и без политики - прыжок-COM-порт бессмыслен"
        );
        let serial = profile(&[json!({ "id": "c", "name": "стойка", "connection": "serial", "port": "COM3" })]);
        assert!(resolve_with("c", serial, &Policy::default())
            .err()
            .unwrap()
            .contains("COM-порт"));
    }

    #[test]
    fn proxy_command_под_ограничивающей_политикой_запрещён() {
        let p = profile(&[json!({ "id": "t", "host": "10.0.0.5", "proxyCommand": "ncat 192.168.1.1 22" })]);
        assert!(
            resolve_with("t", &p, &Policy::default()).is_ok(),
            "без политики - как раньше"
        );
        let e = resolve_with("t", &p, &allowed(&["10.0.0.0/24"])).err().unwrap();
        assert!(e.contains("ProxyCommand"), "{e}");
        let no_shell = Policy {
            forbid_local_terminal: true,
            ..Default::default()
        };
        assert!(resolve_with("t", &p, &no_shell).is_err());
        let blank = profile(&[json!({ "id": "t", "host": "10.0.0.5", "proxyCommand": "  " })]);
        assert!(
            resolve_with("t", blank, &allowed(&["10.0.0.0/24"])).is_ok(),
            "пустая команда - не команда"
        );
    }

    #[test]
    fn запрет_сохранённых_паролей_снимает_их_со_всех_звеньев() {
        let p = profile(&[
            json!({ "id": "t", "host": "a", "password": "1", "proxyJump": "j" }),
            json!({ "id": "j", "host": "b", "passphrase": "2" }),
        ]);
        let no_saved = Policy {
            forbid_saved_passwords: true,
            ..Default::default()
        };
        let chain = resolve_with("t", p, &no_saved).unwrap();
        assert!(chain
            .iter()
            .all(|s| s.get("password").is_none() && s.get("passphrase").is_none()));
    }
}
