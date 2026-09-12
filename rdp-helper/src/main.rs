//! Помощник RDP для Serein.
//!
//! Отдельный процесс, а не модуль приложения, по одной причине: IronRDP через `picky`
//! прибивает знаком `=` шестнадцать крейтов RustCrypto к release candidate'ам, а SSH-ядро
//! приложения использует те же шестнадцать в стабильных версиях. В одном дереве
//! зависимостей они не сходятся - проверено тремя способами, включая подмену через
//! `[patch]` и откат `russh` на версию с той же линией.
//!
//! Из этого вынужденного решения выходит и польза: упавший декодер чужого протокола не
//! роняет приложение, а его зависимости не смешиваются с теми, на которых держится SSH.
//!
//! Подключается помощник не к серверу напрямую, а к локальному сокету, который держит
//! приложение: за ним уже стоит канал `direct-tcpip` внутри живой SSH-сессии. Поэтому
//! наружу здесь ничего не открывается, и никакой своей сетевой политики у помощника нет.
//!
//! Запуск: `serein-rdp --port <локальный порт> --user <имя> [--domain <домен>]
//!          [--width W] [--height H] [--color-depth 32|24|16] [--network-profile vpn|lan]
//!          [--economy] [--no-autologon]`,
//! пароль - первой строкой стандартного ввода.
//! Пароль не берётся доводом намеренно: строка запуска видна в списке процессов.

mod proto;

use std::io::BufRead;

use ironrdp_async::FramedWrite;
use tokio::io::{AsyncRead, AsyncWrite};

/// Поток, по которому можно и читать, и писать.
///
/// Свой трейт, потому что у IronRDP такой же объявлен внутри их клиента и наружу не
/// выведен. Нужен ради одного: после подъёма TLS тип потока меняется, а дальше код
/// одинаков для обоих случаев, и держать его в боксе за трейтом дешевле, чем разводить
/// две копии цикла сеанса.
trait AsyncReadWrite: AsyncRead + AsyncWrite {}
impl<T> AsyncReadWrite for T where T: AsyncRead + AsyncWrite {}

/// Заглушка сетевого клиента для KDC.
///
/// Он нужен `connect_finalize` только ради Kerberos: сходить к контроллеру домена за
/// билетом. Мы ходим внутри SSH-сессии к одной машине, доменного Kerberos на этом пути
/// нет, и тащить ради него `reqwest` со своим TLS в помощника - лишние мегабайты и
/// лишняя сетевая поверхность. NTLM, которым RDP пользуется в остальных случаях, KDC
/// не требует вовсе. Отказ здесь внятный: он попадёт в текст ошибки соединения.
struct NoKdc;

impl ironrdp_async::NetworkClient for NoKdc {
    async fn send(
        &mut self,
        _request: &ironrdp::connector::sspi::generator::NetworkRequest,
    ) -> ironrdp::connector::ConnectorResult<Vec<u8>> {
        Err(ironrdp::connector::custom_err!(
            "Kerberos через контроллер домена в этой сборке не поддержан",
            std::io::Error::other("войдите по паролю: RDP тогда использует NTLM")
        ))
    }
}

use ironrdp::connector::{self, ClientConnector, ConnectionResult, Credentials, DesktopSize};
use ironrdp::input::{Database, MouseButton, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp::pdu::gcc::KeyboardType;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::session::{ActiveStageBuilder, ActiveStageOutput};

/// Разобранная строка запуска. Всё, кроме пароля: тот приходит по входу.
struct Args {
    port: u16,
    user: String,
    domain: Option<String>,
    width: u16,
    height: u16,
    /// Бит на точку: 32 - как есть, 16 - вчетверо меньше трафика и заметные полосы
    /// на плавных переходах.
    color_depth: u32,
    /// Экономия оформления: без обоев, тем, анимации меню и перетаскивания окон целиком.
    economy: bool,
    /// Отдавать ли серверу готовый вход. Выключено - сервер спросит имя и пароль сам.
    autologon: bool,
    /// Профиль полосы, которую сервер учитывает при выборе оформления и кодирования.
    network_profile: NetworkProfile,
}

#[derive(Clone, Copy)]
enum NetworkProfile {
    Vpn,
    Lan,
}

fn parse_args() -> Result<Args, String> {
    let mut port = None;
    let mut user = String::new();
    let mut domain = None;
    let (mut width, mut height) = (1280u16, 800u16);
    let mut color_depth = 32u32;
    let mut economy = false;
    let mut autologon = true;
    let mut network_profile = NetworkProfile::Vpn;

    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => port = it.next().and_then(|v| v.parse().ok()),
            "--user" => user = it.next().unwrap_or_default(),
            "--domain" => domain = it.next().filter(|d| !d.is_empty()),
            "--width" => width = it.next().and_then(|v| v.parse().ok()).unwrap_or(width),
            "--height" => height = it.next().and_then(|v| v.parse().ok()).unwrap_or(height),
            // Чужое число здесь опаснее умолчания: сервер на невозможную глубину цвета
            // отвечает разрывом, а не подсказкой. Поэтому берём только известные.
            "--color-depth" => {
                color_depth = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|d| [16u32, 24, 32].contains(d))
                    .unwrap_or(color_depth)
            }
            "--economy" => economy = true,
            "--no-autologon" => autologon = false,
            "--network-profile" => {
                network_profile = match it.next().as_deref() {
                    Some("vpn") => NetworkProfile::Vpn,
                    Some("lan") => NetworkProfile::Lan,
                    Some(other) => return Err(format!("неизвестный профиль сети: {other}")),
                    None => return Err("не задано значение --network-profile".to_owned()),
                }
            }
            other => return Err(format!("неизвестный довод: {other}")),
        }
    }

    Ok(Args {
        port: port.ok_or("не задан --port")?,
        user,
        domain,
        width,
        height,
        color_depth,
        economy,
        autologon,
        network_profile,
    })
}

fn config(a: &Args, password: String) -> connector::Config {
    use ironrdp::pdu::rdp::client_info::PerformanceFlags;

    // Экономия оформления. Отключаем именно украшения: обои, темы, анимацию меню и
    // перерисовку окна целиком при перетаскивании. Работать это не мешает, а трафика
    // на медленном канале экономит больше всего остального вместе взятого.
    let performance_flags = if a.economy {
        PerformanceFlags::DISABLE_WALLPAPER
            | PerformanceFlags::DISABLE_THEMING
            | PerformanceFlags::DISABLE_MENUANIMATIONS
            | PerformanceFlags::DISABLE_FULLWINDOWDRAG
            | PerformanceFlags::DISABLE_CURSOR_SHADOW
    } else {
        PerformanceFlags::default()
    };

    connector::Config {
        desktop_size: DesktopSize {
            width: a.width,
            height: a.height,
        },
        desktop_scale_factor: 0,
        // TLS обязателен, NLA тоже: без них современный Windows соединение не примет,
        // а разрешать откат на старую защиту RDP значило бы предлагать худший вариант
        // молча.
        enable_tls: true,
        enable_credssp: true,
        credentials: Credentials::UsernamePassword {
            username: a.user.clone(),
            password,
        },
        domain: a.domain.clone(),
        client_build: 0,
        client_name: "serein".to_owned(),
        keyboard_type: KeyboardType::IBM_ENHANCED,
        keyboard_subtype: 0,
        keyboard_functional_keys_count: 12,
        keyboard_layout: 0,
        ime_file_name: String::new(),
        // RemoteFX выключен намеренно. На живой проверке против xrdp сервер выбирал
        // именно его, и разобранный кадр приходил скошенным: каждая строка смещена на
        // два пикселя относительно предыдущей. Искажение сидит уже в самом кадре, до
        // нашей вырезки области - это проверено выгрузкой буфера целиком.
        //
        // Без RFX сервер шлёт обычные битмапы: трафика больше, зато картинка верная.
        // Правильно и медленнее лучше, чем быстро и криво.
        bitmap: Some(connector::BitmapConfig {
            color_depth: a.color_depth,
            // В VPN-профиле разрешаем серверу пропускать альфу, снижать точность цвета
            // и применять subsampling. Для текста это почти незаметно, зато полоса при
            // прокрутке и перетаскивании окон падает существенно.
            lossy_compression: matches!(a.network_profile, NetworkProfile::Vpn),
            codecs: ironrdp::pdu::rdp::capability_sets::client_codecs_capabilities(&[
                "remotefx:off",
            ])
            .unwrap_or_else(|_| {
                ironrdp::pdu::rdp::capability_sets::client_codecs_capabilities(&[])
                    .expect("пустой список кодеков разбирается всегда")
            }),
        }),
        dig_product_id: String::new(),
        client_dir: String::new(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        platform: MajorPlatformType::UNSPECIFIED,
        hardware_id: None,
        request_data: None,
        // Имя и пароль уже спрошены нашей формой и уходят серверу в сведениях о клиенте.
        // Без этого признака сервер их не применяет и показывает поверх картинки ещё
        // одно окно входа - вторые те же самые имя с паролем подряд. Признак сообщает:
        // это готовый вход, спрашивать заново не надо.
        autologon: a.autologon,
        enable_audio_playback: false,
        performance_flags,
        license_cache: None,
        timezone_info: Default::default(),
        // Сжатие внутри RDP (MPPC) выключено, и это не экономия на мелочах. Включённое, оно
        // ломает смену размера: после неё сервер присылает пакеты пересогласования уже
        // сжатыми, а разбор пересогласования в IronRDP сжатых пакетов не понимает и рвёт
        // сеанс с «unexpected server message». Пропускать их тоже нельзя - история
        // разжимателя общая на весь сеанс, и следующий кадр развалился бы.
        //
        // Сжатие при этом есть, только уровнем ниже: в профиле VPN соединение рабочего
        // стола идёт своим SSH с zlib, и оно сжимает весь поток целиком.
        compression_type: None,
        enable_server_pointer: true,
        pointer_software_rendering: false,
        multitransport_flags: None,
        // Сервер видит локальный конец SSH-моста, но реальная полоса остаётся полосой
        // VPN. Если соврать ему про LAN, он выбирает качество, способное забить туннель.
        connection_type: match a.network_profile {
            NetworkProfile::Vpn => ironrdp::pdu::gcc::ConnectionType::BroadbandLow,
            NetworkProfile::Lan => ironrdp::pdu::gcc::ConnectionType::Lan,
        },
        // Старую защиту RDP не предлагаем: у нас есть TLS и NLA, а без них соединение
        // было бы слабее ровно там, где идут нажатия клавиш.
        enable_standard_rdp_security: false,
        // Звук в обе стороны пока не поддержан - ни воспроизведение, ни запись.
        enable_audio_capture: false,
        // RemoteApp: запуск отдельного окна вместо рабочего стола. Нам нужен стол.
        remote_application_mode: false,
        rail_support_level: ironrdp::pdu::rdp::capability_sets::RailSupportLevel::empty(),
        // Графический канал поверх динамических виртуальных каналов. Не включаем: он
        // тянет свой конвейер кадров, а обычного пути нам сейчас достаточно.
        support_dyn_vc_gfx_protocol: false,
        // Раскладка мониторов: один экран, размер которого мы уже задали выше.
        monitor_layout: None,
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut out = std::io::stdout();

    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            proto::send_closed(&mut out, &e);
            std::process::exit(2);
        }
    };

    // Пароль первой строкой входа. В доводы он не попадает: строка запуска процесса
    // видна в системе всем, кто может смотреть список процессов.
    let mut password = String::new();
    if std::io::stdin().lock().read_line(&mut password).is_err() {
        proto::send_closed(&mut out, "не удалось прочитать пароль со входа");
        std::process::exit(2);
    }
    let password = password.trim_end_matches(['\r', '\n']).to_owned();

    // Дальше в вывод пишет только поток выдачи: два писателя в одну трубу перемешали бы
    // байты кадров, и приложение потеряло бы границы пакетов.
    drop(out);
    let out = Presenter::spawn(match args.network_profile {
        NetworkProfile::Vpn => Some(72),
        NetworkProfile::Lan => None,
    });
    let result = run(args, password, &out).await;
    if let Err(e) = &result {
        out.packet(proto::packet(proto::KIND_CLOSED, 0, 0, 0, 0, e.as_bytes()));
    }
    out.finish();
    if result.is_err() {
        std::process::exit(1);
    }
}

async fn run(a: Args, password: String, out: &Presenter) -> Result<(), String> {
    // Приложение уже держит этот сокет: за ним канал внутри SSH-сессии.
    let sock = tokio::net::TcpStream::connect(("127.0.0.1", a.port))
        .await
        .map_err(|e| format!("не подключиться к локальному сокету {}: {e}", a.port))?;
    sock.set_nodelay(true).ok();

    let mut framed = ironrdp_tokio::TokioFramed::new(sock);
    // Канал управления экраном объявляем сразу: договориться о нём можно только при
    // подключении, а нужен он позже - когда окно поменяет размер.
    let dvc = ironrdp::dvc::DrdynvcClient::new().with_dynamic_channel(
        ironrdp::displaycontrol::client::DisplayControlClient::new(|_| Ok(Vec::new())),
    );
    let mut connector = ClientConnector::new(config(&a, password), ([127, 0, 0, 1], a.port).into())
        .with_static_channel(dvc);

    let should_upgrade = ironrdp_tokio::connect_begin(&mut framed, &mut connector)
        .await
        .map_err(|e| format!("рукопожатие RDP не прошло: {e}"))?;

    // TLS поднимаем их же помощником, а не своим: он заодно достаёт открытый ключ
    // сервера в том виде, в каком его ждёт CredSSP для привязки канала.
    //
    // Имя для проверки сертификата тут условное, и это осознанно: RDP-серверы почти
    // всегда представляются самоподписанным сертификатом, а подлинность стороны здесь
    // обеспечивает SSH-сессия, внутри которой идёт весь трафик. Требовать доверенный
    // сертификат значило бы не пускать никуда вообще.
    let (initial_stream, leftover) = framed.into_inner();
    let (tls_stream, tls_cert) = ironrdp_tls::upgrade(initial_stream, "serein")
        .await
        .map_err(|e| format!("TLS не установился: {e}"))?;

    let server_public_key = ironrdp_tls::extract_tls_server_public_key(&tls_cert)
        .ok_or("из сертификата сервера не достать открытый ключ")?
        .to_owned();

    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);
    let erased: Box<dyn AsyncReadWrite + Unpin + Send + Sync> = Box::new(tls_stream);
    let mut upgraded_framed = ironrdp_tokio::TokioFramed::new_with_leftover(erased, leftover);

    let connection = ironrdp_tokio::connect_finalize(
        upgraded,
        connector,
        &mut upgraded_framed,
        &mut NoKdc,
        ironrdp::connector::ServerName::new("serein"),
        server_public_key,
        None,
    )
    .await
    .map_err(|e| format!("соединение не установилось: {e}"))?;

    let frame_period = match a.network_profile {
        NetworkProfile::Vpn => std::time::Duration::from_millis(33),
        NetworkProfile::Lan => std::time::Duration::from_millis(16),
    };
    session(connection, upgraded_framed, out, frame_period).await
}

/// Основной цикл: кадры наружу, команды внутрь.
async fn session(
    connection: ConnectionResult,
    framed: ironrdp_tokio::TokioFramed<Box<dyn AsyncReadWrite + Unpin + Send + Sync>>,
    out: &Presenter,
    frame_period: std::time::Duration,
) -> Result<(), String> {
    // Делим поток надвое: чтение держит свою половину всё время ожидания кадра, и
    // отправить в неё ответ на нажатие клавиши в этот момент было бы нечем.
    let (mut framed, mut writer) = ironrdp_tokio::split_tokio_framed(framed);
    // Фабрика пересогласования нужна после каждой смены размера: сервер отвечает на неё
    // полным пересбором сеанса, и пройти эту последовательность обязан клиент.
    let activation_factory = connection.activation_factory;
    let w = connection.desktop_size.width;
    let h = connection.desktop_size.height;
    out.packet(proto::packet(proto::KIND_RESIZE, 0, 0, w, h, &[]));

    let mut image = ironrdp::session::image::DecodedImage::new(
        ironrdp::graphics::image_processing::PixelFormat::RgbA32,
        w,
        h,
    );
    let mut stage = ActiveStageBuilder {
        static_channels: connection.static_channels,
        user_channel_id: connection.user_channel_id,
        io_channel_id: connection.io_channel_id,
        message_channel_id: connection.message_channel_id,
        share_id: connection.share_id,
        compression_type: connection.compression_type,
        enable_server_pointer: connection.enable_server_pointer,
        pointer_software_rendering: connection.pointer_software_rendering,
    }
    .build();

    // Стандартный вход блокирующий, поэтому читает его отдельный поток: в рантайме
    // он подвесил бы всё на первом же ожидании строки.
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<proto::Cmd>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if let Some(c) = proto::parse_cmd(&line) {
                let quit = c == proto::Cmd::Quit;
                if cmd_tx.send(c).is_err() || quit {
                    break;
                }
            }
        }
    });

    // Состояние клавиатуры и мыши держит сама библиотека: она же гасит команды, которые
    // ничего не меняют, - лишние нажатия одной и той же клавиши на сервер не поедут.
    let mut input_db = Database::new();
    // Сервер часто присылает один визуальный кадр десятками прямоугольников. Если
    // немедленно протолкнуть каждый через stdout и IPC, очередь растёт быстрее экрана.
    // Сохраняем все области в локальном framebuffer, а наружу выдаём пачку 30 раз/с для
    // VPN и до 60 раз/с для LAN. Перекрывающиеся области объединяются без потери пикселей.
    let mut dirty = DirtyRegions::default();
    let mut frame_tick = tokio::time::interval(frame_period);
    frame_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    frame_tick.tick().await;

    // Пока поток команд жив, его слушаем. Закрылся - просто перестаём слушать: сеанс
    // от этого не заканчивается. Живая проверка против xrdp показала, почему это важно:
    // при закрытом вводе помощник уходил через секунду после соединения, а сервер в это
    // время уже кодировал экран входа.
    let mut cmds_open = true;

    loop {
        let payload = tokio::select! {
            // Поток выдачи ещё занят прошлой пачкой - области копятся дальше и уйдут
            // следующим тиком одной пачкой. Кадров в секунду становится меньше, но
            // показывается всегда самое свежее, а сеть читается без остановки.
            _ = frame_tick.tick(), if !dirty.is_empty() => {
                if !out.is_busy() {
                    let regions = dirty
                        .take()
                        .into_iter()
                        .map(|r| (r, crop(&image, r.left, r.top, r.width(), r.height())))
                        .collect();
                    out.regions(regions);
                }
                None
            }
            frame = framed.read_pdu() => {
                let (action, payload) = frame
                    .map_err(|e| format!("связь с сервером прервалась: {e}"))?;
                Some((action, payload))
            }
            cmd = cmd_rx.recv(), if cmds_open => {
                let Some(cmd) = cmd else {
                    cmds_open = false;
                    continue;
                };
                match cmd {
                    proto::Cmd::Quit => return Ok(()),
                    proto::Cmd::Resize { w, h } => {
                        // Предел протокола: от 200 до 8192 и чётная ширина. Подгоняем
                        // молча - окно тянут мышью, а не набирают числами.
                        let (rw, rh) =
                            ironrdp::displaycontrol::pdu::MonitorLayoutEntry::adjust_display_size(
                                u32::from(w),
                                u32::from(h),
                            );
                        // Тот же размер после подгонки - просить не о чем. Каждая просьба
                        // стоит серверу полной пересборки сеанса, а картинка на это время
                        // замирает.
                        if (rw, rh) == (u32::from(image.width()), u32::from(image.height())) {
                            continue;
                        }
                        // Состояние канала называем словами: молча пропущенная просьба
                        // о новом размере выглядит как «ничего не произошло», и отличить
                        // «сервер не умеет» от «мы не отправили» по ней невозможно.
                        match stage.display_control_ready() {
                            Some(true) => {}
                            Some(false) => eprintln!(
                                "канал управления экраном ещё не готов - размер не сменить"
                            ),
                            None => eprintln!(
                                "сервер не поддержал канал управления экраном - размер не сменить"
                            ),
                        }
                        // `None` - сервер канал не поддержал. Это не поломка: картинка
                        // просто останется вписанной в окно, как было раньше всегда.
                        if let Some(res) = stage.encode_resize(rw, rh, None, None) {
                            let bytes =
                                res.map_err(|e| format!("смена размера не собралась: {e}"))?;
                            writer
                                .write_all(&bytes)
                                .await
                                .map_err(|e| format!("смена размера не ушла: {e}"))?;
                        }
                    }
                    // Весь экран целиком - по просьбе приложения. Так выглядит переезд
                    // сеанса в другое окно: у нового окна пустой холст, а неподвижный
                    // рабочий стол сам по себе не пришлёт ничего.
                    proto::Cmd::Full => {
                        dirty.clear();
                        let (fw, fh) = (image.width(), image.height());
                        out.packet(proto::packet(proto::KIND_RESIZE, 0, 0, fw, fh, &[]));
                        dirty.add(Rect::from_xywh(0, 0, fw, fh));
                    }
                    other => {
                        let events = input_db.apply(to_operations(other));
                        if !events.is_empty() {
                            let outs = stage
                                .process_fastpath_input(&mut image, &events)
                                .map_err(|e| format!("ввод не отправился: {e}"))?;
                            for o in outs {
                                if let ActiveStageOutput::ResponseFrame(f) = o {
                                    writer
                                        .write_all(&f)
                                        .await
                                        .map_err(|e| format!("ввод не ушёл на сервер: {e}"))?;
                                }
                            }
                        }
                    }
                }
                None
            }
        };

        let Some((action, payload)) = payload else {
            continue;
        };

        let outputs = stage
            .process(&mut image, action, &payload)
            .map_err(|e| format!("кадр не разобрался: {e}"))?;

        for o in outputs {
            match o {
                // Сервер принял новый размер и пересобирает сеанс. Пока эта
                // последовательность не пройдена, обычных кадров не будет вовсе.
                ActiveStageOutput::DeactivateAll => {
                    dirty.clear();
                    let (nw, nh) = reactivate(
                        &activation_factory,
                        &mut framed,
                        &mut writer,
                        &mut stage,
                        &mut image,
                    )
                    .await?;
                    image = ironrdp::session::image::DecodedImage::new(
                        ironrdp::graphics::image_processing::PixelFormat::RgbA32,
                        nw,
                        nh,
                    );
                    out.packet(proto::packet(proto::KIND_RESIZE, 0, 0, nw, nh, &[]));
                }
                ActiveStageOutput::ResponseFrame(f) => {
                    writer
                        .write_all(&f)
                        .await
                        .map_err(|e| format!("ответ не ушёл на сервер: {e}"))?;
                }
                ActiveStageOutput::GraphicsUpdate(region) => {
                    let (x, y) = (region.left, region.top);
                    let rw = region.right.saturating_sub(region.left) + 1;
                    let rh = region.bottom.saturating_sub(region.top) + 1;
                    dirty.add(Rect::from_xywh(x, y, rw, rh));
                }
                ActiveStageOutput::Terminate(reason) => {
                    return Err(format!("сервер завершил сеанс: {reason}"));
                }
                _ => {}
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    left: u16,
    top: u16,
    right: u16,
    bottom: u16,
}

impl Rect {
    fn from_xywh(x: u16, y: u16, w: u16, h: u16) -> Self {
        Self {
            left: x,
            top: y,
            right: x.saturating_add(w.saturating_sub(1)),
            bottom: y.saturating_add(h.saturating_sub(1)),
        }
    }

    fn width(self) -> u16 {
        self.right.saturating_sub(self.left) + 1
    }

    fn height(self) -> u16 {
        self.bottom.saturating_sub(self.top) + 1
    }

    fn touches(self, other: Self) -> bool {
        u32::from(self.left) <= u32::from(other.right) + 1
            && u32::from(other.left) <= u32::from(self.right) + 1
            && u32::from(self.top) <= u32::from(other.bottom) + 1
            && u32::from(other.top) <= u32::from(self.bottom) + 1
    }

    fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
}

#[derive(Default)]
struct DirtyRegions(Vec<Rect>);

impl DirtyRegions {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn clear(&mut self) {
        self.0.clear();
    }

    fn take(&mut self) -> Vec<Rect> {
        std::mem::take(&mut self.0)
    }

    fn add(&mut self, mut incoming: Rect) {
        let mut i = 0;
        while i < self.0.len() {
            if incoming.touches(self.0[i]) {
                incoming = incoming.union(self.0.swap_remove(i));
                i = 0;
            } else {
                i += 1;
            }
        }
        self.0.push(incoming);

        // Даже патологическая россыпь областей не должна создать неограниченную пачку.
        // При достижении предела один общий прямоугольник дешевле очереди из сотен IPC.
        if self.0.len() >= 32 {
            let all = self
                .0
                .drain(..)
                .reduce(Rect::union)
                .expect("список не пуст");
            self.0.push(all);
        }
    }
}

/// Поток после подъёма TLS. Тип длинный, а встречается трижды.
type Stream = Box<dyn AsyncReadWrite + Unpin + Send + Sync>;

/// Проходит пересогласование после смены размера и возвращает новый размер экрана.
///
/// Сервер на смену размера отвечает последовательностью «отключить всё и собрать заново»:
/// заново объявляются возможности, заново выдаётся идентификатор сеанса. Пропустить её
/// нельзя - до конца этой переписки сервер не пришлёт ни одного кадра.
async fn reactivate(
    factory: &ironrdp::connector::connection_activation::ConnectionActivationFactory,
    framed: &mut ironrdp_tokio::TokioFramed<tokio::io::ReadHalf<Stream>>,
    writer: &mut ironrdp_tokio::TokioFramed<tokio::io::WriteHalf<Stream>>,
    stage: &mut ironrdp::session::ActiveStage,
    image: &mut ironrdp::session::image::DecodedImage,
) -> Result<(u16, u16), String> {
    use ironrdp::connector::Sequence as _;
    use ironrdp::connector::connection_activation::ConnectionActivationState;

    let fail = |e: &dyn std::fmt::Display| format!("пересогласование после смены размера не прошло: {e}");
    let mut activation = factory.create();
    let io_channel = activation.io_channel_id();
    let mut buf = ironrdp_core::WriteBuf::new();
    loop {
        buf.clear();
        // Шаг за шагом вручную, а не готовым `single_sequence_step_read`: тот отдаёт
        // последовательности всё подряд, а она на любой пакет не из своего списка
        // обрывает сеанс. Сервер же в это время живёт своей жизнью - xrdp, например,
        // успевает прислать состояние клавиатуры или данные виртуальных каналов.
        let written = match activation.next_pdu_hint() {
            Some(hint) => {
                let pdu = framed.read_by_hint(hint).await.map_err(|e| fail(&e))?;
                match classify_during_activation(&pdu, io_channel) {
                    Activation::Step => activation.step(&pdu, None, &mut buf).map_err(|e| fail(&e))?,
                    // Данные виртуальных каналов - дело сеанса, а не пересогласования:
                    // канал управления экраном, например, живёт через всю пересборку.
                    Activation::Channel => {
                        match stage.process(image, ironrdp::pdu::Action::X224, &pdu) {
                            Ok(outs) => {
                                for o in outs {
                                    if let ActiveStageOutput::ResponseFrame(f) = o {
                                        writer.write_all(&f).await.map_err(|e| fail(&e))?;
                                    }
                                }
                            }
                            Err(e) => eprintln!("пакет канала во время пересогласования не разобрался: {e}"),
                        }
                        continue;
                    }
                    // Служебное, что к пересогласованию не относится. После него сервер
                    // всё равно перерисует экран целиком, так что терять здесь нечего.
                    Activation::Skip(name) => {
                        eprintln!("во время пересогласования пропущен пакет: {name}");
                        continue;
                    }
                }
            }
            None => activation.step_no_input(&mut buf).map_err(|e| fail(&e))?,
        };
        if written.size().is_some() {
            writer
                .write_all(buf.filled())
                .await
                .map_err(|e| format!("ответ пересогласования не ушёл: {e}"))?;
        }
        if let ConnectionActivationState::Finalized {
            desktop_size,
            share_id,
            enable_server_pointer,
            pointer_software_rendering,
            static_channel_chunk_size,
            ..
        } = activation.connection_activation_state()
        {
            if !stage.reactivate(
                activation.io_channel_id(),
                activation.user_channel_id(),
                share_id,
                enable_server_pointer,
                pointer_software_rendering,
                static_channel_chunk_size,
            ) {
                return Err("сервер назвал недопустимый размер куска канала".to_owned());
            }
            return Ok((desktop_size.width, desktop_size.height));
        }
    }
}

/// Что делать с пакетом, пришедшим посреди пересогласования.
#[derive(Debug, PartialEq)]
enum Activation {
    /// Отдать последовательности пересогласования.
    Step,
    /// Данные виртуального канала - обработать сеансом.
    Channel,
    /// Постороннее служебное сообщение - пропустить, назвав его в журнале.
    Skip(String),
}

/// Разбирает пакет ровно настолько, чтобы понять, чей он.
///
/// Последовательности пересогласования отдаём всё, что она умеет разобрать, и всё, что
/// не удалось опознать: пусть лучше она внятно откажет, чем мы молча проглотим важное.
/// Пропускаем только то, что опознано и заведомо не её.
fn classify_during_activation(pdu: &[u8], io_channel: u16) -> Activation {
    use ironrdp::pdu::rdp::headers::{ShareControlPdu, ShareDataPdu};

    let Ok(ctx) = ironrdp::pdu::mcs::decode_send_data_indication(pdu) else {
        return Activation::Step;
    };
    if ctx.channel_id != io_channel {
        return Activation::Channel;
    }
    let Ok(share) = ironrdp::pdu::rdp::headers::decode_share_control(ctx) else {
        return Activation::Step;
    };
    match share.pdu {
        ShareControlPdu::Data(header) => match header.share_data_pdu {
            // Это пересогласование знает. Сжатое тоже отдаём ему: раз сжатие выключено,
            // такой пакет - поломка, и о ней надо сказать, а не пропустить его.
            ShareDataPdu::Synchronize(_)
            | ShareDataPdu::Control(_)
            | ShareDataPdu::FontMap(_)
            | ShareDataPdu::MonitorLayout(_)
            | ShareDataPdu::ServerSetErrorInfo(_)
            | ShareDataPdu::Compressed { .. } => Activation::Step,
            other => Activation::Skip(other.as_short_name().to_owned()),
        },
        _ => Activation::Step,
    }
}

/// Переводит нашу команду в события ввода RDP.
///
/// Кнопки мыши: 1 левая, 2 правая, 4 средняя - те же биты, что приходят из браузера,
/// поэтому на стороне интерфейса ничего пересчитывать не нужно.
///
/// Клавиши приходят уже кодами RDP: раскладку и различение левого и правого Alt
/// разбирает интерфейс, у него для этого есть событие браузера целиком, а здесь от него
/// остались бы только домыслы.
fn to_operations(cmd: proto::Cmd) -> Vec<Operation> {
    match cmd {
        proto::Cmd::Pointer { x, y, buttons } => {
            let mut ops = vec![Operation::MouseMove(MousePosition { x, y })];
            for (bit, btn) in [
                (1u8, MouseButton::Left),
                (2, MouseButton::Right),
                (4, MouseButton::Middle),
            ] {
                ops.push(if buttons & bit != 0 {
                    Operation::MouseButtonPressed(btn)
                } else {
                    Operation::MouseButtonReleased(btn)
                });
            }
            ops
        }
        proto::Cmd::Key { code, down } => {
            let sc = Scancode::from_u16(code);
            vec![if down {
                Operation::KeyPressed(sc)
            } else {
                Operation::KeyReleased(sc)
            }]
        }
        proto::Cmd::Wheel { vertical, delta } => vec![Operation::WheelRotations(WheelRotations {
            is_vertical: vertical,
            rotation_units: delta,
        })],
        // Эти разбираются раньше, до перевода в события ввода: смена размера идёт
        // своим каналом, полный кадр - это вообще не ввод, а выход заканчивает сеанс.
        proto::Cmd::Resize { .. } | proto::Cmd::Full | proto::Cmd::Quit => Vec::new(),
    }
}

/// Вырезает прямоугольник из полного кадра.
///
/// Обновления приходят областями, а не целым экраном - в этом весь смысл: пересылать
/// восемь мегабайт на каждое движение курсора нельзя.
fn crop(image: &ironrdp::session::image::DecodedImage, x: u16, y: u16, w: u16, h: u16) -> Vec<u8> {
    let stride = usize::from(image.width()) * 4;
    let data = image.data();
    let mut out = Vec::with_capacity(usize::from(w) * usize::from(h) * 4);
    for row in 0..usize::from(h) {
        let start = (usize::from(y) + row) * stride + usize::from(x) * 4;
        let end = start + usize::from(w) * 4;
        if end <= data.len() {
            out.extend_from_slice(&data[start..end]);
        } else {
            out.resize(out.len() + usize::from(w) * 4, 0);
        }
    }
    out
}

/// Пакет для интерфейса из уже вырезанной области RGBA.
///
/// Мелкие области остаются RGBA: на них запуск JPEG дороже самих данных. Большие
/// обновления VPN-профиля кодируются в JPEG, чтобы мегабайты пикселей не копировались
/// несколько раз между помощником, Tauri IPC и WebView.
fn region_packet(rect: Rect, rgba: &[u8], jpeg_quality: Option<u8>) -> Vec<u8> {
    const JPEG_MIN_PIXELS: usize = 256 * 256;
    let pixels = usize::from(rect.width()) * usize::from(rect.height());
    if let Some(quality) = jpeg_quality.filter(|_| pixels >= JPEG_MIN_PIXELS) {
        let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        let mut encoded = Vec::with_capacity(pixels);
        let encoder = jpeg_encoder::Encoder::new(&mut encoded, quality);
        if encoder
            .encode(&rgb, rect.width(), rect.height(), jpeg_encoder::ColorType::Rgb)
            .is_ok()
        {
            return proto::packet(
                proto::KIND_JPEG,
                rect.left,
                rect.top,
                rect.width(),
                rect.height(),
                &encoded,
            );
        }
    }
    proto::packet(proto::KIND_RAW, rect.left, rect.top, rect.width(), rect.height(), rgba)
}

/// Задание потоку выдачи.
enum Job {
    /// Готовый пакет: смена размера, закрытие.
    Packet(Vec<u8>),
    /// Пачка вырезанных областей - упаковать и отдать.
    Regions(Vec<(Rect, Vec<u8>)>),
}

/// Выдача кадров приложению - своим потоком, а не в сетевом цикле.
///
/// Причина измерена, а не придумана. JPEG полного кадра 2370x1248 стоит около 30 мс, а
/// бюджет кадра в профиле VPN - 33 мс. Пока сжатие шло в сетевом цикле, при перетаскивании
/// окна помощник почти всё время жал картинку и почти не читал сокет: данные от сервера
/// копились в локальных буферах, и экран отставал на секунды. На маленьком окне этого не
/// видно - пикселей вчетверо меньше.
///
/// Теперь сетевой цикл только разбирает поток и помечает изменённые области, а здесь их
/// упаковывают и пишут. Пока поток занят, новая пачка не отдаётся: области копятся в
/// цикле и уйдут одной пачкой, когда он освободится. Отстать так нельзя - самый свежий
/// кадр ждёт не дольше одной упаковки.
struct Presenter {
    tx: std::sync::mpsc::Sender<Job>,
    busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}

impl Presenter {
    fn spawn(jpeg_quality: Option<u8>) -> Self {
        use std::sync::atomic::Ordering;
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        let busy = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = busy.clone();
        let thread = std::thread::spawn(move || {
            let mut out = std::io::stdout();
            for job in rx {
                let written = match job {
                    Job::Packet(p) => proto::send(&mut out, &p),
                    Job::Regions(list) => {
                        let r = list.into_iter().try_for_each(|(rect, rgba)| {
                            proto::send(&mut out, &region_packet(rect, &rgba, jpeg_quality))
                        });
                        done.store(false, Ordering::Release);
                        r
                    }
                };
                // Труба закрыта - приложение ушло, и показывать картинку больше некому.
                if written.is_err() {
                    std::process::exit(1);
                }
            }
        });
        Self { tx, busy, thread }
    }

    fn packet(&self, p: Vec<u8>) {
        let _ = self.tx.send(Job::Packet(p));
    }

    fn is_busy(&self) -> bool {
        self.busy.load(std::sync::atomic::Ordering::Acquire)
    }

    fn regions(&self, list: Vec<(Rect, Vec<u8>)>) {
        self.busy.store(true, std::sync::atomic::Ordering::Release);
        let _ = self.tx.send(Job::Regions(list));
    }

    /// Дописывает всё отправленное и закрывает поток. Без этого пакет о закрытии мог бы
    /// не успеть уйти до выхода процесса.
    fn finish(self) {
        drop(self.tx);
        let _ = self.thread.join();
    }
}

#[cfg(test)]
mod activation_tests {
    use super::*;
    use ironrdp::pdu::rdp::headers::{
        CompressionFlags, ShareControlHeader, ShareControlPdu, ShareDataHeader, ShareDataPdu,
        StreamPriority,
    };

    const IO: u16 = 1003;

    /// Пакет от сервера - в том виде, в каком он приходит по сети.
    fn from_server(channel: u16, pdu: ShareDataPdu) -> Vec<u8> {
        let share = ShareControlHeader {
            share_id: 0x1_0000,
            pdu_source: 1002,
            share_control_pdu: ShareControlPdu::Data(ShareDataHeader {
                share_data_pdu: pdu,
                stream_priority: StreamPriority::Undefined,
                compression_flags: CompressionFlags::empty(),
                compression_type: ironrdp::pdu::rdp::client_info::CompressionType::K8,
            }),
        };
        let user_data = ironrdp_core::encode_vec(&share).expect("заголовок собирается");
        ironrdp_core::encode_vec(&ironrdp::pdu::x224::X224(ironrdp::pdu::mcs::SendDataIndication {
            initiator_id: 1002,
            channel_id: channel,
            user_data: std::borrow::Cow::Owned(user_data),
        }))
        .expect("пакет собирается")
    }

    #[test]
    fn постороннее_служебное_пропускается_а_не_рвёт_сеанс() {
        // Такое сообщение пересогласование не знает и раньше обрывало на нём сеанс с
        // «unexpected server message».
        let pdu = from_server(IO, ShareDataPdu::ShutdownDenied);
        assert!(matches!(classify_during_activation(&pdu, IO), Activation::Skip(_)));
    }

    #[test]
    fn своё_пересогласование_получает_как_есть() {
        let pdu = from_server(IO, ShareDataPdu::FontMap(Default::default()));
        assert_eq!(classify_during_activation(&pdu, IO), Activation::Step);
    }

    #[test]
    fn виртуальный_канал_уходит_сеансу() {
        let pdu = from_server(IO + 1, ShareDataPdu::FontMap(Default::default()));
        assert_eq!(classify_during_activation(&pdu, IO), Activation::Channel);
    }

    #[test]
    fn неопознанное_не_глотается_молча() {
        // Пусть лучше пересогласование внятно откажет, чем мы потеряем важный пакет.
        assert_eq!(classify_during_activation(&[3, 0, 0, 4], IO), Activation::Step);
    }
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    #[test]
    fn соседние_области_объединяются_без_потери_краёв() {
        let mut dirty = DirtyRegions::default();
        dirty.add(Rect::from_xywh(10, 20, 5, 4));
        dirty.add(Rect::from_xywh(15, 20, 3, 4));
        assert_eq!(
            dirty.take(),
            vec![Rect {
                left: 10,
                top: 20,
                right: 17,
                bottom: 23,
            }]
        );
    }

    #[test]
    fn раздельные_области_не_раздуваются_в_полный_кадр() {
        let mut dirty = DirtyRegions::default();
        dirty.add(Rect::from_xywh(0, 0, 2, 2));
        dirty.add(Rect::from_xywh(100, 100, 2, 2));
        assert_eq!(dirty.take().len(), 2);
    }

    #[test]
    fn vpn_кодирует_большой_кадр_а_маленький_оставляет_сырым() {
        let image = ironrdp::session::image::DecodedImage::new(
            ironrdp::graphics::image_processing::PixelFormat::RgbA32,
            512,
            512,
        );
        let big = Rect::from_xywh(0, 0, 512, 512);
        let little = Rect::from_xywh(0, 0, 32, 32);
        let large = region_packet(big, &crop(&image, 0, 0, 512, 512), Some(72));
        let small = region_packet(little, &crop(&image, 0, 0, 32, 32), Some(72));
        assert_eq!(large[0], proto::KIND_JPEG);
        assert_eq!(small[0], proto::KIND_RAW);
        assert!(large.len() < 512 * 512 * 4);
    }
}


