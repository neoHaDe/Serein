//! Сборщик метрик против настоящих серверов: команда замера и её разбор.
//!
//! Запуск: `cargo test --test metrics_stand -- --ignored` с переменными из `up.sh`.

mod common;
use common::{rt, Stand};

use serein_lib::{metrics, ssh};

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn замер_с_настоящего_сервера_становится_точкой_истории() {
    // Логика замера переехала из панели обзора в сборщик сессии. Проверяем её там, где
    // она ломается на деле: на полноценном Debian и на Alpine с BusyBox, где у половины
    // команд другие ключи.
    let s = Stand::from_env();
    rt().block_on(async {
        for (имя, порт) in [("debian", s.debian_port), ("alpine", s.alpine_port)] {
            let h = ssh::connect_client(vec![s.by_key(порт)])
                .await
                .expect("подключение к стенду");
            let id = format!("стенд-метрики-{имя}");
            let замер = metrics::sample(&id, &h, None).await.expect("замер");
            assert_eq!(замер["ok"], true, "{имя}: замер не удался: {замер}");

            let точка = metrics::point_of(&замер, metrics::now_ms()).expect("точка истории");
            assert!(точка.cores >= 1, "{имя}: ядер не бывает меньше одного");
            assert!(
                точка.mem > 0.0 && точка.mem <= 100.0,
                "{имя}: память должна прийти процентом, а пришло {}",
                точка.mem
            );
            assert!(точка.load.is_some(), "{имя}: у юникса средняя загрузка есть");
        }
    });
}
