//! Обмен с главным приложением.
//!
//! Кадры уходят в стандартный вывод, ввод приходит со стандартного входа. Формат кадра
//! ровно тот же, что у VNC (`src-tauri/src/vnc.rs` и `src/renderer/vncFrames.ts`), и это
//! не совпадение: интерфейс уже умеет его рисовать, и заводить второй формат ради того же
//! самого значило бы держать два разбора вместо одного.
//!
//! Пакет: `[u8 тип][u16 x][u16 y][u16 w][u16 h][тело]`, порядок байтов сетевой.
//! Поверх этого - длина кадра четырьмя байтами, потому что труба, в отличие от канала
//! Tauri, границ сообщений не хранит.

use std::io::{self, Write};

pub const KIND_RESIZE: u8 = 1;
pub const KIND_RAW: u8 = 2;
pub const KIND_JPEG: u8 = 3;
pub const KIND_CLOSED: u8 = 9;

/// Собирает пакет: заголовок и тело.
pub fn packet(kind: u8, x: u16, y: u16, w: u16, h: u16, body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(9 + body.len());
    v.push(kind);
    v.extend_from_slice(&x.to_be_bytes());
    v.extend_from_slice(&y.to_be_bytes());
    v.extend_from_slice(&w.to_be_bytes());
    v.extend_from_slice(&h.to_be_bytes());
    v.extend_from_slice(body);
    v
}

/// Пишет кадр в вывод с длиной впереди.
///
/// Пишем и сбрасываем сразу: экран должен появляться по мере готовности, а не когда
/// наберётся буфер. При закрытой трубе выходим тихо - значит приложение уже ушло,
/// и жаловаться некому.
pub fn send(out: &mut impl Write, frame: &[u8]) -> io::Result<()> {
    let len = u32::try_from(frame.len()).unwrap_or(u32::MAX);
    out.write_all(&len.to_be_bytes())?;
    out.write_all(frame)?;
    out.flush()
}

/// Сообщает о закрытии и причине. Текст уходит телом пакета.
pub fn send_closed(out: &mut impl Write, reason: &str) {
    let _ = send(out, &packet(KIND_CLOSED, 0, 0, 0, 0, reason.as_bytes()));
}

/// Команда от приложения.
///
/// Строки, а не двоичное: команд мало, они редкие, и читаемый протокол здесь дешевле
/// в отладке, чем экономия десятка байт на нажатие.
#[derive(Debug, PartialEq)]
pub enum Cmd {
    /// `p <x> <y> <кнопки>` - указатель.
    Pointer {
        x: u16,
        y: u16,
        buttons: u8,
    },
    /// `k <код> <1|0>` - клавиша, код в терминах RDP.
    Key {
        code: u16,
        down: bool,
    },
    /// `w <вертикально 1|0> <единицы>` - колесо мыши.
    Wheel {
        vertical: bool,
        delta: i16,
    },
    /// `s <w> <h>` - изменился размер окна.
    Resize {
        w: u16,
        h: u16,
    },
    /// `q` - закрыться.
    /// `f` - прислать весь экран целиком.
    ///
    /// Нужна при переезде сеанса в другое окно: новое окно начинает с пустого холста, а
    /// неподвижный рабочий стол сам по себе не шлёт ничего часами.
    Full,
    Quit,
}

/// Разбирает строку команды. Мусор игнорируем: обрыв протокола не повод падать.
pub fn parse_cmd(line: &str) -> Option<Cmd> {
    let mut it = line.split_whitespace();
    match it.next()? {
        "p" => Some(Cmd::Pointer {
            x: it.next()?.parse().ok()?,
            y: it.next()?.parse().ok()?,
            buttons: it.next()?.parse().ok()?,
        }),
        "k" => Some(Cmd::Key {
            code: it.next()?.parse().ok()?,
            down: it.next()? == "1",
        }),
        "w" => Some(Cmd::Wheel {
            vertical: it.next()? == "1",
            delta: it.next()?.parse().ok()?,
        }),
        "s" => Some(Cmd::Resize {
            w: it.next()?.parse().ok()?,
            h: it.next()?.parse().ok()?,
        }),
        "f" => Some(Cmd::Full),
        "q" => Some(Cmd::Quit),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn заголовок_пакета_совпадает_с_тем_что_ждёт_интерфейс() {
        // Девять байт заголовка и сетевой порядок - разбор на стороне интерфейса
        // (`vncFrames.ts`) устроен ровно так, и расхождение здесь рисует мусор.
        let p = packet(KIND_RAW, 1, 2, 3, 4, &[9, 9]);
        assert_eq!(p.len(), 9 + 2);
        assert_eq!(p[0], KIND_RAW);
        assert_eq!(&p[1..3], &1u16.to_be_bytes());
        assert_eq!(&p[7..9], &4u16.to_be_bytes());
        assert_eq!(&p[9..], &[9, 9]);
    }

    #[test]
    fn кадр_уходит_с_длиной_впереди() {
        // Труба границ сообщений не хранит, поэтому длина обязательна.
        let mut out = Vec::new();
        send(&mut out, &[7, 7, 7]).unwrap();
        assert_eq!(&out[..4], &3u32.to_be_bytes());
        assert_eq!(&out[4..], &[7, 7, 7]);
    }

    #[test]
    fn команды_разбираются() {
        assert_eq!(
            parse_cmd("p 10 20 1"),
            Some(Cmd::Pointer {
                x: 10,
                y: 20,
                buttons: 1
            })
        );
        assert_eq!(
            parse_cmd("k 65 1"),
            Some(Cmd::Key {
                code: 65,
                down: true
            })
        );
        assert_eq!(
            parse_cmd("k 65 0"),
            Some(Cmd::Key {
                code: 65,
                down: false
            })
        );
        assert_eq!(
            parse_cmd("w 1 -120"),
            Some(Cmd::Wheel {
                vertical: true,
                delta: -120
            })
        );
        assert_eq!(
            parse_cmd("w 0 80"),
            Some(Cmd::Wheel {
                vertical: false,
                delta: 80
            })
        );
        assert_eq!(parse_cmd("s 800 600"), Some(Cmd::Resize { w: 800, h: 600 }));
        assert_eq!(parse_cmd("f"), Some(Cmd::Full));
        assert_eq!(parse_cmd("q"), Some(Cmd::Quit));
    }

    #[test]
    fn мусор_не_роняет_разбор() {
        // По трубе может прийти что угодно, включая обрывок при закрытии. Падать на этом
        // нельзя: помощник обязан пережить кривой ввод и закрыться сам.
        assert_eq!(parse_cmd(""), None);
        assert_eq!(parse_cmd("p"), None);
        assert_eq!(parse_cmd("p 10"), None);
        assert_eq!(parse_cmd("p a b c"), None);
        assert_eq!(parse_cmd("невнятица"), None);
    }
}
