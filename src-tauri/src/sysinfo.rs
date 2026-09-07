//! Железо сервера: процессор, видео, память, виртуализация.
//!
//! Отдельно от `monitor.rs` потому, что это разные по природе вещи. Загрузка меняется
//! каждую секунду и опрашивается по таймеру; модель процессора не меняется никогда, и
//! спрашивать её тридцать раз в минуту — впустую гонять канал. Здесь всё собирается один
//! раз за сессию.
//!
//! Главное правило разбора: **чего не смогли узнать — о том говорим словами**. Прочерк без
//! объяснения заставляет человека гадать, сломалось ли приложение или у него правда нет
//! такой железки. Скорость памяти, например, без `dmidecode` и прав root не достать вовсе,
//! и это надо сказать, а не показать пустоту.

use serde_json::{json, Value};

/// Одна команда на весь снимок: каждый `exec` — отдельный круг по сети.
///
/// Видео ищется через `/sys`, а не через `lspci`: последнего на минимальных серверах нет,
/// а класс устройства и ссылка на драйвер в `/sys` есть всегда. `lspci` используется
/// только чтобы перевести числовой идентификатор в человеческое название — если он есть.
pub const CMD: &str = concat!(
    "echo \"CPU:$(grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2- | sed 's/^ *//')\"; ",
    "echo \"CORES:$(nproc 2>/dev/null)\"; ",
    "echo \"THREADS:$(grep -c '^processor' /proc/cpuinfo 2>/dev/null)\"; ",
    "echo \"MHZ:$(grep -m1 'cpu MHz' /proc/cpuinfo 2>/dev/null | cut -d: -f2- | tr -d ' ')\"; ",
    "for d in /sys/bus/pci/devices/*/; do ",
    "  c=$(cat \"$d/class\" 2>/dev/null); ",
    "  case \"$c\" in 0x0300*|0x0302*) ",
    "    s=$(basename \"$d\"); ",
    "    v=$(basename \"$(readlink -f \"$d/driver\" 2>/dev/null)\" 2>/dev/null); ",
    "    n=''; ",
    "    if command -v lspci >/dev/null 2>&1; then ",
    "      n=$(lspci -s \"$s\" 2>/dev/null | sed 's/^[^ ]* //; s/^[^:]*: //'); ",
    "    fi; ",
    "    if [ -z \"$n\" ]; then ",
    "      n=\"$(cat \"$d/vendor\" 2>/dev/null):$(cat \"$d/device\" 2>/dev/null)\"; ",
    "    fi; ",
    "    echo \"GPU:$n|$v\"; ",
    "  ;; esac; ",
    "done; ",
    // Скорость и тип памяти лежат в таблицах DMI, а их читает только root и только через
    // dmidecode. Оба условия выполняются далеко не везде, поэтому причина отказа
    // сообщается отдельной строкой — человеку важно знать, чего именно не хватило.
    "if ! command -v dmidecode >/dev/null 2>&1; then echo 'MEMWHY:на сервере нет dmidecode'; ",
    "elif ! dmidecode -t 17 >/dev/null 2>&1; then echo 'MEMWHY:нужны права root'; ",
    "else dmidecode -t 17 2>/dev/null | awk '",
    "/Size:/{sz=$2\" \"$3} ",
    "/Type:/{if($2!=\"Unknown\")tp=$2} ",
    "/Speed:/{if($2!=\"Unknown\"&&sp==\"\")sp=$2\" \"$3} ",
    "END{if(sp!=\"\"||tp!=\"\")print \"MEM:\"sp\"|\"tp}'; fi; ",
    "if command -v systemd-detect-virt >/dev/null 2>&1; then echo \"VIRT:$(systemd-detect-virt 2>/dev/null)\"; ",
    "elif grep -q hypervisor /proc/cpuinfo 2>/dev/null; then echo 'VIRT:виртуальная машина'; ",
    "else echo 'VIRT:none'; fi"
);

/// Человеческое имя производителя по числовому коду PCI.
///
/// Без `lspci` в ответе остаются голые идентификаторы вида `0x1002:0x1638`, а по ним
/// человек не поймёт ничего. Полная база кодов весит мегабайты и живёт в `pci.ids` — тащить
/// её в приложение ради одной строки не стоит, но пяти вендоров, которые делают
/// подавляющее большинство видеоядер, хватает, чтобы надпись стала осмысленной.
fn vendor_name(code: &str) -> Option<&'static str> {
    match code.trim_start_matches("0x").to_lowercase().as_str() {
        "1002" => Some("AMD/ATI"),
        "10de" => Some("NVIDIA"),
        "8086" => Some("Intel"),
        "1a03" => Some("ASPEED"),
        "102b" => Some("Matrox"),
        "1234" => Some("QEMU"),
        "15ad" => Some("VMware"),
        "1414" => Some("Microsoft"),
        _ => None,
    }
}

/// Приводит имя видеоустройства к читаемому виду.
///
/// Если это пара идентификаторов — переводим вендора и оставляем код устройства: точную
/// модель без базы не назвать, но «AMD/ATI, устройство 1638» несравнимо полезнее, чем
/// «0x1002:0x1638».
fn gpu_name(raw: &str) -> String {
    let raw = raw.trim();
    if let Some((v, d)) = raw.split_once(':') {
        if v.starts_with("0x") && d.starts_with("0x") {
            let d = d.trim_start_matches("0x");
            return match vendor_name(v) {
                Some(name) => format!("{name}, устройство {d}"),
                None => format!("{}, устройство {d}", v.trim_start_matches("0x")),
            };
        }
    }
    raw.to_string()
}

/// Разбирает ответ в вид, понятный панели обзора.
pub fn parse(stdout: &str) -> Value {
    let mut cpu = String::new();
    let mut cores = 0u32;
    let mut threads = 0u32;
    let mut mhz = 0f64;
    let mut gpus: Vec<Value> = Vec::new();
    let mut mem_speed = String::new();
    let mut mem_type = String::new();
    let mut mem_why = String::new();
    let mut virt = String::new();

    for line in stdout.lines() {
        let Some((tag, val)) = line.split_once(':') else { continue };
        let val = val.trim();
        match tag.trim() {
            "CPU" => cpu = val.to_string(),
            "CORES" => cores = val.parse().unwrap_or(0),
            "THREADS" => threads = val.parse().unwrap_or(0),
            "MHZ" => mhz = val.parse().unwrap_or(0.0),
            "GPU" => {
                let (name, driver) = val.split_once('|').unwrap_or((val, ""));
                let name = name.trim();
                if name.is_empty() {
                    continue;
                }
                gpus.push(json!({
                    "name": gpu_name(name),
                    // Пустая строка и «драйвер не загружен» — разные вещи. Второе бывает
                    // и означает, что видео работает на базовом режиме без ускорения.
                    "driver": if driver.trim().is_empty() { Value::Null } else { json!(driver.trim()) },
                }));
            }
            "MEM" => {
                let (s, t) = val.split_once('|').unwrap_or((val, ""));
                mem_speed = s.trim().to_string();
                mem_type = t.trim().to_string();
            }
            "MEMWHY" => mem_why = val.to_string(),
            "VIRT" => virt = val.to_string(),
            _ => {}
        }
    }

    let mut out = json!({ "gpus": gpus });
    if !cpu.is_empty() {
        out["cpu"] = json!(cpu);
    }
    if cores > 0 {
        out["cores"] = json!(cores);
    }
    // Потоки показываем только когда их больше ядер: иначе это то же число дважды.
    if threads > cores {
        out["threads"] = json!(threads);
    }
    if mhz > 0.0 {
        out["mhz"] = json!(mhz.round() as u32);
    }
    if !mem_speed.is_empty() {
        out["memSpeed"] = json!(mem_speed);
    }
    if !mem_type.is_empty() {
        out["memType"] = json!(mem_type);
    }
    if !mem_why.is_empty() && mem_speed.is_empty() {
        out["memWhy"] = json!(mem_why);
    }
    // «none» означает железо, и говорить об этом отдельной строкой незачем — интересен
    // только сам факт виртуализации.
    if !virt.is_empty() && virt != "none" {
        out["virt"] = json!(virt);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn разбирает_обычный_ответ() {
        let out = concat!(
            "CPU:AMD Ryzen 7 5800H with Radeon Graphics\n",
            "CORES:16\nTHREADS:16\nMHZ:3193.884\n",
            "GPU:Advanced Micro Devices, Inc. [AMD/ATI] Cezanne [Radeon Vega Series]|amdgpu\n",
            "VIRT:none\n"
        );
        let v = parse(out);
        assert_eq!(v["cpu"], "AMD Ryzen 7 5800H with Radeon Graphics");
        assert_eq!(v["cores"], 16);
        assert_eq!(v["mhz"], 3194);
        assert_eq!(v["gpus"][0]["driver"], "amdgpu");
        // Железная машина — строки про виртуализацию быть не должно.
        assert!(v.get("virt").is_none());
    }

    #[test]
    fn потоки_не_дублируют_ядра() {
        // На машине без многопоточности это одно и то же число, и показывать его дважды
        // значит засорять экран.
        let v = parse("CORES:8\nTHREADS:8\n");
        assert!(v.get("threads").is_none());
        let v = parse("CORES:8\nTHREADS:16\n");
        assert_eq!(v["threads"], 16);
    }

    #[test]
    fn причина_отсутствия_скорости_памяти_доходит_до_панели() {
        // Прочерк без объяснения заставляет гадать, сломалось приложение или нет.
        let v = parse("CPU:x\nMEMWHY:нужны права root\n");
        assert_eq!(v["memWhy"], "нужны права root");
        assert!(v.get("memSpeed").is_none());
    }

    #[test]
    fn когда_скорость_известна_причина_не_нужна() {
        let v = parse("MEM:3200 MT/s|DDR4\nMEMWHY:нужны права root\n");
        assert_eq!(v["memSpeed"], "3200 MT/s");
        assert_eq!(v["memType"], "DDR4");
        assert!(v.get("memWhy").is_none(), "причина рядом с ответом сбивает с толку");
    }

    #[test]
    fn видео_без_драйвера_отличается_от_видео_без_имени() {
        // Незагруженный драйвер — законное состояние: видео работает без ускорения.
        // А вот запись без имени показывать нечего, её отбрасываем.
        let v = parse("GPU:Какая-то видеокарта|\nGPU:|nouveau\n");
        let g = v["gpus"].as_array().unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0]["name"], "Какая-то видеокарта");
        assert!(g[0]["driver"].is_null());
    }

    #[test]
    fn без_lspci_имя_остаётся_осмысленным() {
        // На минимальных серверах `lspci` нет, и в ответе приходят голые коды. «AMD/ATI,
        // устройство 1638» человек прочитает, «0x1002:0x1638» — нет.
        let v = parse("GPU:0x1002:0x1638|amdgpu\n");
        assert_eq!(v["gpus"][0]["name"], "AMD/ATI, устройство 1638");
        assert_eq!(v["gpus"][0]["driver"], "amdgpu");

        let v = parse("GPU:0x10de:0x2504|nvidia\n");
        assert_eq!(v["gpus"][0]["name"], "NVIDIA, устройство 2504");
    }

    #[test]
    fn незнакомый_вендор_не_выдумывается() {
        // Придумать название по коду нельзя — показываем сам код, но в читаемом виде.
        let v = parse("GPU:0xabcd:0x1234|drv\n");
        assert_eq!(v["gpus"][0]["name"], "abcd, устройство 1234");
    }

    #[test]
    fn готовое_имя_от_lspci_не_трогаем() {
        let v = parse("GPU:Advanced Micro Devices, Inc. [AMD/ATI] Cezanne|amdgpu\n");
        assert_eq!(v["gpus"][0]["name"], "Advanced Micro Devices, Inc. [AMD/ATI] Cezanne");
    }

    #[test]
    fn виртуализация_называется_своим_именем() {
        assert_eq!(parse("VIRT:kvm\n")["virt"], "kvm");
        assert_eq!(parse("VIRT:microsoft\n")["virt"], "microsoft");
    }

    #[test]
    fn пустой_ответ_не_роняет_разбор() {
        let v = parse("");
        assert_eq!(v["gpus"].as_array().unwrap().len(), 0);
        assert!(v.get("cpu").is_none());
    }
}
