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
//!          [--width W] [--height H] [--color-depth 32|24|16] [--economy] [--no-autologon]`,
//! пароль - первой строкой стандартного ввода.
//! Пароль не берётся доводом намеренно: строка запуска видна в списке процессов.

mod proto;

use std::io::{BufRead, Write};

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

use ironrdp::connector::{
    self, ClientConnector, ConnectionResult, Credentials, DesktopSize,
};
use ironrdp::pdu::gcc::KeyboardType;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::input::{Database, MouseButton, MousePosition, Operation, Scancode};
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
}

fn parse_args() -> Result<Args, String> {
    let mut port = None;
    let mut user = String::new();
    let mut domain = None;
    let (mut width, mut height) = (1280u16, 800u16);
    let mut color_depth = 32u32;
    let mut economy = false;
    let mut autologon = true;

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
        desktop_size: DesktopSize { width: a.width, height: a.height },
        desktop_scale_factor: 0,
        // TLS обязателен, NLA тоже: без них современный Windows соединение не примет,
        // а разрешать откат на старую защиту RDP значило бы предлагать худший вариант
        // молча.
        enable_tls: true,
        enable_credssp: true,
        credentials: Credentials::UsernamePassword { username: a.user.clone(), password },
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
            lossy_compression: false,
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
        compression_type: None,
        enable_server_pointer: true,
        pointer_software_rendering: false,
        multitransport_flags: None,
        // Соединение объявляем как локальную сеть: мы и правда ходим внутри SSH-канала
        // до машины рядом, а не через модем. От этого сервер выбирает, чем жертвовать
        // ради скорости - при «медленном» соединении он отключил бы часть оформления.
        connection_type: ironrdp::pdu::gcc::ConnectionType::Lan,
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

    if let Err(e) = run(args, password, &mut out).await {
        proto::send_closed(&mut out, &e);
        std::process::exit(1);
    }
}

async fn run(a: Args, password: String, out: &mut impl Write) -> Result<(), String> {
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

    session(connection, upgraded_framed, out).await
}

/// Основной цикл: кадры наружу, команды внутрь.
async fn session(
    connection: ConnectionResult,
    framed: ironrdp_tokio::TokioFramed<Box<dyn AsyncReadWrite + Unpin + Send + Sync>>,
    out: &mut impl Write,
) -> Result<(), String> {
    // Делим поток надвое: чтение держит свою половину всё время ожидания кадра, и
    // отправить в неё ответ на нажатие клавиши в этот момент было бы нечем.
    let (mut framed, mut writer) = ironrdp_tokio::split_tokio_framed(framed);
    // Фабрика пересогласования нужна после каждой смены размера: сервер отвечает на неё
    // полным пересбором сеанса, и пройти эту последовательность обязан клиент.
    let activation_factory = connection.activation_factory;
    let w = connection.desktop_size.width;
    let h = connection.desktop_size.height;
    proto::send(out, &proto::packet(proto::KIND_RESIZE, 0, 0, w, h, &[]))
        .map_err(|e| format!("приложение не читает вывод: {e}"))?;

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

    // Пока поток команд жив, его слушаем. Закрылся - просто перестаём слушать: сеанс
    // от этого не заканчивается. Живая проверка против xrdp показала, почему это важно:
    // при закрытом вводе помощник уходил через секунду после соединения, а сервер в это
    // время уже кодировал экран входа.
    let mut cmds_open = true;

    loop {
        let payload = tokio::select! {
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
                        let (fw, fh) = (image.width(), image.height());
                        proto::send(out, &proto::packet(proto::KIND_RESIZE, 0, 0, fw, fh, &[]))
                            .map_err(|e| format!("приложение не читает вывод: {e}"))?;
                        let px = crop(&image, 0, 0, fw, fh);
                        proto::send(out, &proto::packet(proto::KIND_RAW, 0, 0, fw, fh, &px))
                            .map_err(|e| format!("приложение не читает вывод: {e}"))?;
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

        let Some((action, payload)) = payload else { continue };

        let outputs = stage
            .process(&mut image, action, &payload)
            .map_err(|e| format!("кадр не разобрался: {e}"))?;

        for o in outputs {
            match o {
                // Сервер принял новый размер и пересобирает сеанс. Пока эта
                // последовательность не пройдена, обычных кадров не будет вовсе.
                ActiveStageOutput::DeactivateAll => {
                    let (nw, nh) =
                        reactivate(&activation_factory, &mut framed, &mut writer, &mut stage).await?;
                    image = ironrdp::session::image::DecodedImage::new(
                        ironrdp::graphics::image_processing::PixelFormat::RgbA32,
                        nw,
                        nh,
                    );
                    proto::send(out, &proto::packet(proto::KIND_RESIZE, 0, 0, nw, nh, &[]))
                        .map_err(|e| format!("приложение не читает вывод: {e}"))?;
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
                    // Порядок каналов менять не нужно: выше мы просим раскодировать
                    // прямо в RGBA, а холст ждёт именно его. Лишняя перестановка здесь
                    // однажды уже была, и синее показывалось красным.
                    let px = crop(&image, x, y, rw, rh);
                    proto::send(out, &proto::packet(proto::KIND_RAW, x, y, rw, rh, &px))
                        .map_err(|e| format!("приложение не читает вывод: {e}"))?;
                }
                ActiveStageOutput::Terminate(reason) => {
                    return Err(format!("сервер завершил сеанс: {reason}"));
                }
                _ => {}
            }
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
) -> Result<(u16, u16), String> {
    use ironrdp::connector::connection_activation::ConnectionActivationState;

    let mut activation = factory.create();
    let mut buf = ironrdp_core::WriteBuf::new();
    loop {
        let written = ironrdp_async::single_sequence_step_read(framed, &mut activation, &mut buf)
            .await
            .map_err(|e| format!("пересогласование после смены размера не прошло: {e}"))?;
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
            vec![if down { Operation::KeyPressed(sc) } else { Operation::KeyReleased(sc) }]
        }
        // Эти разбираются раньше, до перевода в события ввода: смена размера идёт
        // своим каналом, полный кадр - это вообще не ввод, а выход заканчивает сеанс.
        proto::Cmd::Resize { .. } | proto::Cmd::Full | proto::Cmd::Quit => Vec::new(),
    }
}

/// Вырезает прямоугольник из полного кадра.
///
/// Обновления приходят областями, а не целым экраном - в этом весь смысл: пересылать
/// восемь мегабайт на каждое движение курсора нельзя.
fn crop(
    image: &ironrdp::session::image::DecodedImage,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
) -> Vec<u8> {
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
