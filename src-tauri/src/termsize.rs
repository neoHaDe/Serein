//! Размер окна терминала, который присылает интерфейс.
//!
//! Одно правило на все виды сессий. Раньше их было четыре: у локального терминала пределов
//! не было вовсе, у смены размера - только нижний, у Telnet - оба, у SSH - никаких. А
//! приведение `u64 as u16` молча заворачивало большое число: 65 536 столбцов становились нулём.

use serde_json::Value;

/// Меньше этого терминал непригоден, больше - это уже не окно, а ошибка в расчёте.
pub const COLS: (u16, u16) = (20, 500);
pub const ROWS: (u16, u16) = (5, 200);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
}

fn field(p: &Value, key: &str) -> Option<u64> {
    p.get(key).and_then(|v| v.as_u64())
}

fn clamp(v: u64, (lo, hi): (u16, u16)) -> u16 {
    v.clamp(u64::from(lo), u64::from(hi)) as u16
}

/// Размер для открытия сессии: нет поля - 80×24, выход за пределы - к ближайшей границе.
pub fn for_open(p: &Value) -> TermSize {
    TermSize {
        cols: clamp(field(p, "cols").unwrap_or(80), COLS),
        rows: clamp(field(p, "rows").unwrap_or(24), ROWS),
    }
}

/// Размер для смены. Слишком маленькое окно - это свёрнутая или ещё не разложенная панель:
/// такую смену пропускаем, иначе программа на сервере перерисуется в пять столбцов. Слишком
/// большое - обрезаем до границы.
pub fn for_resize(p: &Value) -> Option<TermSize> {
    let cols = field(p, "cols").unwrap_or(80);
    let rows = field(p, "rows").unwrap_or(24);
    if cols < u64::from(COLS.0) || rows < u64::from(ROWS.0) {
        return None;
    }
    Some(TermSize {
        cols: clamp(cols, COLS),
        rows: clamp(rows, ROWS),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn без_размера_открывается_80_на_24() {
        assert_eq!(for_open(&json!({})), TermSize { cols: 80, rows: 24 });
        assert_eq!(
            for_open(&json!({ "cols": "много", "rows": -3 })),
            TermSize { cols: 80, rows: 24 }
        );
    }

    #[test]
    fn большой_размер_обрезается_а_не_заворачивается() {
        // `65536 as u16` - это ноль, `70000 as u16` - 4464.
        assert_eq!(
            for_open(&json!({ "cols": 65536, "rows": 70000 })),
            TermSize { cols: 500, rows: 200 }
        );
        assert_eq!(
            for_resize(&json!({ "cols": 65536, "rows": 70000 })),
            Some(TermSize { cols: 500, rows: 200 })
        );
        assert_eq!(
            for_open(&json!({ "cols": 3, "rows": 1 })),
            TermSize { cols: 20, rows: 5 }
        );
    }

    #[test]
    fn смена_на_крошечное_окно_пропускается() {
        assert_eq!(for_resize(&json!({ "cols": 0, "rows": 0 })), None);
        assert_eq!(for_resize(&json!({ "cols": 19, "rows": 40 })), None);
        assert_eq!(for_resize(&json!({ "cols": 120, "rows": 4 })), None);
        assert_eq!(
            for_resize(&json!({ "cols": 120, "rows": 40 })),
            Some(TermSize { cols: 120, rows: 40 })
        );
    }
}
