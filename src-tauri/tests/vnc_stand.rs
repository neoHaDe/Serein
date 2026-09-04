//! Рабочий стол VNC через SSH-канал.
//!
//! В стенде сервер `x11vnc` запущен с `-localhost`: снаружи контейнера его порт недоступен
//! в принципе. Это не искусственное ограничение, а обычная практика — своя защита у VNC
//! слабая, поэтому его держат за SSH. Значит, единственный способ дойти до экрана — канал
//! `direct-tcpip` внутри уже установленной SSH-сессии, и тест проверяет именно его.
//!
//! Запуск: `cargo test --test vnc_stand -- --ignored --test-threads=1`

mod common;
use common::{rt, Stand};

use serein_lib::ssh;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Открывает канал до рабочего стола так же, как это делает приложение.
async fn desktop_channel(s: &Stand) -> russh::ChannelStream<russh::client::Msg> {
    let h = ssh::connect_client(vec![s.by_key(s.vnc_port)])
        .await
        .expect("подключение к серверу с рабочим столом");
    let ch = {
        let g = h.lock().await;
        // 127.0.0.1 здесь — петля сервера, а не наша: канал открывает удалённая сторона.
        g.channel_open_direct_tcpip("127.0.0.1", 5900, "127.0.0.1", 0)
            .await
            .expect("канал до 127.0.0.1:5900 на сервере")
    };
    ch.into_stream()
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn desktop_is_reachable_only_through_the_ssh_channel() {
    let s = Stand::from_env();
    rt().block_on(async {
        // Прямое подключение обязано провалиться: порт наружу не опубликован. Если этот
        // шаг когда-нибудь пройдёт — значит стенд перестал проверять то, ради чего он есть.
        let direct = tokio::time::timeout(
            Duration::from_secs(3),
            tokio::net::TcpStream::connect((s.host.as_str(), 5900)),
        )
        .await;
        assert!(
            !matches!(direct, Ok(Ok(_))),
            "порт 5900 виден снаружи — стенд больше не проверяет путь через SSH"
        );

        let mut stream = desktop_channel(&s).await;
        let mut hello = [0u8; 12];
        stream.read_exact(&mut hello).await.expect("приветствие RFB");
        let text = String::from_utf8_lossy(&hello).to_string();
        assert!(
            text.starts_with("RFB 003."),
            "ожидали приветствие RFB, получили «{}»",
            text.escape_debug()
        );
    });
}

#[test]
#[ignore = "нужен стенд: scripts/ssh-stand/up.sh"]
fn server_offers_password_authentication() {
    // Проверка рукопожатия до ввода пароля: сервер должен предложить VncAuth (тип 2).
    // Если он вдруг предложит «без пароля», незамеченным это остаться не должно.
    let s = Stand::from_env();
    rt().block_on(async {
        let mut stream = desktop_channel(&s).await;
        let mut hello = [0u8; 12];
        stream.read_exact(&mut hello).await.expect("приветствие");
        stream.write_all(b"RFB 003.008\n").await.expect("наша версия");

        let mut count = [0u8; 1];
        stream.read_exact(&mut count).await.expect("число типов защиты");
        assert!(count[0] > 0, "сервер отказал в рукопожатии");

        let mut types = vec![0u8; count[0] as usize];
        stream.read_exact(&mut types).await.expect("список типов");
        assert!(
            types.contains(&2),
            "ожидали VncAuth (2), сервер предлагает {types:?}"
        );
    });
}
