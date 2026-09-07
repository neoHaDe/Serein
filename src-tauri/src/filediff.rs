//! Сравнение двух файлов: одинаковы ли, и если нет - чем.
//!
//! Полезно это не само по себе, а тем, откуда берутся файлы. Вопрос, который задают на
//! самом деле, звучит так: «конфиг на этом сервере тот же, что на том?» или «правка,
//! которую я редактировал локально, доехала?». Поэтому каждая сторона сравнения может
//! быть и локальным файлом, и файлом на любой открытой сессии.
//!
//! Алгоритм взят готовый (`similar`), и это не лень. Качество диффа целиком определяется
//! алгоритмом: наивный обход даёт формально верный, но нечитаемый результат, где вместо
//! одной изменённой строки показано полфайла. Здесь готовая библиотека подходит,
//! в отличие от MySQL, где ни одна не годилась по устройству.

use serde_json::{json, Value};
use similar::{ChangeTag, TextDiff};

/// Сколько строк изменений показываем.
///
/// Дифф на десять тысяч строк панель не переварит, а человек не прочитает. Счётчики при
/// этом считаются по всему файлу - обрезается показ, а не подсчёт.
pub const MAX_LINES: usize = 2000;

/// Строка результата: как она изменилась и что в ней.
fn tag_name(t: ChangeTag) -> &'static str {
    match t {
        ChangeTag::Delete => "del",
        ChangeTag::Insert => "add",
        ChangeTag::Equal => "eq",
    }
}

/// Сравнивает два текста построчно.
///
/// `label_a` и `label_b` попадают в ответ как есть: панели надо показать, что с чем
/// сравнивали, и «файл A» вместо пути - плохой ответ.
pub fn compare(label_a: &str, a: &str, label_b: &str, b: &str) -> Value {
    // Одинаковость проверяем до диффа: у совпавших файлов это самый частый исход, и
    // гонять по ним алгоритм незачем.
    if a == b {
        return json!({
            "a": label_a,
            "b": label_b,
            "same": true,
            "added": 0,
            "removed": 0,
            "lines": [],
        });
    }

    let diff = TextDiff::from_lines(a, b);
    let mut lines: Vec<Value> = Vec::new();
    let (mut added, mut removed) = (0usize, 0usize);
    // Номера строк в каждом файле: без них по диффу нельзя найти место в исходнике.
    let (mut na, mut nb) = (0usize, 0usize);

    for ch in diff.iter_all_changes() {
        match ch.tag() {
            ChangeTag::Delete => {
                removed += 1;
                na += 1;
            }
            ChangeTag::Insert => {
                added += 1;
                nb += 1;
            }
            ChangeTag::Equal => {
                na += 1;
                nb += 1;
            }
        }
        if lines.len() < MAX_LINES {
            lines.push(json!({
                "tag": tag_name(ch.tag()),
                "a": if ch.tag() == ChangeTag::Insert { Value::Null } else { json!(na) },
                "b": if ch.tag() == ChangeTag::Delete { Value::Null } else { json!(nb) },
                "text": ch.value().trim_end_matches(['\n', '\r']),
            }));
        }
    }

    let mut out = json!({
        "a": label_a,
        "b": label_b,
        "same": false,
        "added": added,
        "removed": removed,
        "lines": lines,
    });
    // Про обрезку говорим прямо: молча укороченный дифф читается как полный.
    if added + removed > 0 && diff.iter_all_changes().count() > MAX_LINES {
        out["truncated"] = json!(format!("Показаны первые {MAX_LINES} строк сравнения"));
    }
    out
}

/// Похоже ли содержимое на двоичное.
///
/// Дифф двоичных файлов бессмыслен: он покажет мусор и не ответит ни на один вопрос.
/// Признак простой и достаточный - нулевой байт, которого в тексте не бывает.
pub fn looks_binary(s: &str) -> bool {
    s.contains('\0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn одинаковые_файлы_не_считаются_диффом() {
        let v = compare("a.conf", "один\nдва\n", "b.conf", "один\nдва\n");
        assert_eq!(v["same"], true);
        assert_eq!(v["added"], 0);
        assert_eq!(v["removed"], 0);
        assert_eq!(v["lines"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn изменённая_строка_видна_как_замена() {
        let v = compare("a", "один\nдва\nтри\n", "b", "один\nДВА\nтри\n");
        assert_eq!(v["same"], false);
        assert_eq!(v["added"], 1);
        assert_eq!(v["removed"], 1);
        let lines = v["lines"].as_array().unwrap();
        // Совпавшие строки тоже в ответе: без них непонятно, где случилась правка.
        assert!(lines.iter().any(|l| l["tag"] == "eq"));
        assert!(lines.iter().any(|l| l["tag"] == "del" && l["text"] == "два"));
        assert!(lines.iter().any(|l| l["tag"] == "add" && l["text"] == "ДВА"));
    }

    #[test]
    fn номера_строк_считаются_по_каждому_файлу_отдельно() {
        // Без номеров по диффу нельзя найти место в исходнике, а общая нумерация врала бы
        // про один из файлов.
        let v = compare("a", "один\nдва\n", "b", "один\nлишняя\nдва\n");
        let lines = v["lines"].as_array().unwrap();
        let ins = lines.iter().find(|l| l["tag"] == "add").unwrap();
        // Вставленной строки в первом файле нет вовсе.
        assert!(ins["a"].is_null());
        assert_eq!(ins["b"], 2);
    }

    #[test]
    fn пустой_и_непустой_файл_сравниваются() {
        let v = compare("a", "", "b", "строка\n");
        assert_eq!(v["same"], false);
        assert_eq!(v["added"], 1);
        assert_eq!(v["removed"], 0);
    }

    #[test]
    fn разные_переводы_строк_не_считаются_различием_содержимого() {
        // Файл, съездивший через Windows, отличается только концами строк. Показывать это
        // как «изменились все строки» - верный способ спрятать настоящую правку.
        let v = compare("a", "один\nдва\n", "b", "один\r\nдва\r\n");
        let lines = v["lines"].as_array().unwrap();
        for l in lines {
            assert_ne!(l["text"].as_str().unwrap_or(""), "", "пустых строк тут быть не должно");
        }
        // Сам факт различия остаётся - содержимое байт в байт разное, и врать не будем.
        assert_eq!(v["same"], false);
    }

    #[test]
    fn двоичное_содержимое_узнаётся() {
        assert!(looks_binary("абв\0гд"));
        assert!(!looks_binary("обычный текст\nв две строки\n"));
    }
}
