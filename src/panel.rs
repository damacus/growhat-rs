//! Grow HAT Mini display and four input buttons. No pump, buzzer, or light sensor lines are used.
use crate::sensor::SensorReading;
use anyhow::{Result, bail};
use serde::Serialize;
#[cfg(any(target_os = "linux", test))]
use std::net::Ipv4Addr;
#[cfg(any(target_os = "linux", test))]
use std::time::Duration;
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
    thread::JoinHandle,
};

#[cfg(any(target_os = "linux", test))]
const WIDTH: usize = 160;
#[cfg(any(target_os = "linux", test))]
const HEIGHT: usize = 80;
#[cfg(any(target_os = "linux", test))]
const BUTTONS: [(&str, u32); 4] = [("A", 5), ("B", 6), ("X", 16), ("Y", 24)];
#[cfg(any(target_os = "linux", test))]
const DEBOUNCE: Duration = Duration::from_millis(30);

#[derive(Clone, Debug)]
pub enum PanelCommand {
    Message { text: String, ttl_seconds: u64 },
    Readings(Vec<SensorReading>),
    VirtualPress { button: String },
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ButtonEvent {
    pub button: String,
    pub pressed: bool,
    pub count: u64,
}

pub struct Panel {
    pub commands: std::sync::mpsc::SyncSender<PanelCommand>,
    pub events: tokio::sync::mpsc::Receiver<ButtonEvent>,
    pub worker: JoinHandle<Result<()>>,
}

#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Default)]
struct ButtonState {
    raw: bool,
    stable: bool,
    changed_at: Duration,
    count: u64,
}

#[cfg(any(target_os = "linux", test))]
impl ButtonState {
    fn observe(&mut self, button: &str, pressed: bool, now: Duration) -> Option<ButtonEvent> {
        if self.raw != pressed {
            self.raw = pressed;
            self.changed_at = now;
        }
        if self.stable != self.raw && now.saturating_sub(self.changed_at) >= DEBOUNCE {
            self.stable = self.raw;
            if self.stable {
                self.count += 1;
            }
            return Some(ButtonEvent {
                button: button.into(),
                pressed: self.stable,
                count: self.count,
            });
        }
        None
    }
}

#[cfg(any(target_os = "linux", test))]
fn button_snapshots(buttons: &[ButtonState; 4]) -> [ButtonEvent; 4] {
    std::array::from_fn(|index| ButtonEvent {
        button: BUTTONS[index].0.into(),
        pressed: buttons[index].stable,
        count: buttons[index].count,
    })
}

#[cfg(any(target_os = "linux", test))]
fn virtual_press(buttons: &mut [ButtonState; 4], button: &str) -> Option<ButtonEvent> {
    let index = BUTTONS.iter().position(|(name, _)| *name == button)?;
    let state = &mut buttons[index];
    state.count = state.count.saturating_add(1);
    Some(ButtonEvent {
        button: button.into(),
        pressed: state.stable,
        count: state.count,
    })
}

#[cfg(any(target_os = "linux", test))]
#[derive(Clone)]
struct ScreenState {
    readings: Vec<SensorReading>,
    buttons: [ButtonState; 4],
    message: Option<String>,
    sample_number: u64,
    ip_address: Option<Ipv4Addr>,
}

#[cfg(any(target_os = "linux", test))]
impl Default for ScreenState {
    fn default() -> Self {
        Self {
            readings: Vec::new(),
            buttons: std::array::from_fn(|_| ButtonState::default()),
            message: None,
            sample_number: 0,
            ip_address: None,
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn display_lines(state: &ScreenState) -> [String; 4] {
    let address = state
        .ip_address
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "unavailable".to_owned());
    let mut lines = [
        format!("IP {address} #{:04}", state.sample_number % 10_000),
        String::new(),
        String::new(),
        String::new(),
    ];
    for channel in 1..=3 {
        let value = state.readings.iter().find(|r| r.channel == channel);
        lines[channel as usize] = match value {
            Some(r) if r.status == crate::sensor::ReadingStatus::Valid => format!(
                "M{channel}: {:.0}%  {:.1}Hz",
                r.moisture_percent.unwrap_or(0.0),
                r.raw_hz.unwrap_or(0.0)
            ),
            Some(r) if r.status == crate::sensor::ReadingStatus::Uncalibrated => {
                format!("M{channel}: {:.1}Hz  no cal", r.raw_hz.unwrap_or(0.0))
            }
            Some(r) if r.status == crate::sensor::ReadingStatus::NoSignal => {
                format!("M{channel}: NO SIGNAL")
            }
            Some(r) => format!("M{channel}: {:?}", r.status).to_uppercase(),
            None => format!("M{channel}: --"),
        };
    }
    lines
}

/// Whether a test message can be shown in full above the button row.
/// The MQTT handler can reject longer messages before acknowledging them.
pub fn message_fits(text: &str) -> bool {
    !layout_message(text).1
}

fn layout_message(text: &str) -> (Vec<String>, bool) {
    let mut lines = vec![String::new()];
    let mut overflow = false;
    for ch in text.chars() {
        if ch == '\n' || lines.last().is_some_and(|line| line.chars().count() == 24) {
            if lines.len() == 5 {
                overflow = true;
                break;
            }
            lines.push(String::new());
            if ch == '\n' {
                continue;
            }
        }
        if !ch.is_control() {
            lines.last_mut().unwrap().push(ch);
        }
    }
    if overflow {
        let last = lines.last_mut().unwrap();
        for _ in 0..3 {
            last.pop();
        }
        last.push_str("...");
    }
    (lines, overflow)
}

#[cfg(any(target_os = "linux", test))]
fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 & 0xf8) << 8) | ((g as u16 & 0xfc) << 3) | (b as u16 >> 3)
}

#[cfg(any(target_os = "linux", test))]
fn rotated_rgb565_bytes(pixels: &[u16]) -> Vec<u8> {
    let mut data = Vec::with_capacity(WIDTH * HEIGHT * 2);
    // Python vendor image_to_data uses np.rot90(image, 3) for rotation=270.
    for physical_y in 0..160 {
        for physical_x in 0..80 {
            let logical_x = physical_y;
            let logical_y = 79 - physical_x;
            data.extend_from_slice(&pixels[logical_y * WIDTH + logical_x].to_be_bytes());
        }
    }
    data
}

#[cfg(any(target_os = "linux", test))]
fn render(state: &ScreenState) -> Vec<u16> {
    let mut pixels = vec![rgb565(8, 20, 28); WIDTH * HEIGHT];
    let lines = if let Some(message) = &state.message {
        layout_message(message).0
    } else {
        display_lines(state).to_vec()
    };
    for (i, line) in lines.iter().enumerate() {
        draw_text(
            &mut pixels,
            5,
            if state.message.is_some() {
                i * 10
            } else {
                3 + i * 12
            },
            line,
            if i == 0 { rgb565(0, 220, 150) } else { 0xffff },
        );
    }
    for (i, (label, _)) in BUTTONS.iter().enumerate() {
        let button = &state.buttons[i];
        let fill = if button.stable {
            rgb565(0, 220, 150)
        } else {
            rgb565(30, 55, 65)
        };
        fill_rect(&mut pixels, i * 40 + 1, 51, 38, 28, fill);
        let ink = if button.stable { 0 } else { 0xffff };
        draw_text(&mut pixels, i * 40 + 16, 55, label, ink);
        draw_text(&mut pixels, i * 40 + 13, 67, &button.count.to_string(), ink);
    }
    pixels
}

#[cfg(any(target_os = "linux", test))]
fn fill_rect(pixels: &mut [u16], x: usize, y: usize, width: usize, height: usize, colour: u16) {
    for row in y..(y + height).min(HEIGHT) {
        for col in x..(x + width).min(WIDTH) {
            pixels[row * WIDTH + col] = colour;
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn draw_text(pixels: &mut [u16], x: usize, y: usize, text: &str, colour: u16) {
    for (index, ch) in text.chars().take((WIDTH.saturating_sub(x)) / 6).enumerate() {
        let glyph = glyph(ch.to_ascii_uppercase());
        for (row, bits) in glyph.into_iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    let px = x + index * 6 + col;
                    let py = y + row;
                    if px < WIDTH && py < HEIGHT {
                        pixels[py * WIDTH + px] = colour;
                    }
                }
            }
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn glyph(ch: char) -> [u8; 7] {
    match ch {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 14],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [31, 4, 4, 4, 4, 4, 31],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ':' => [0, 4, 4, 0, 4, 4, 0],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        '%' => [17, 18, 4, 8, 19, 17, 0],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '/' => [1, 2, 2, 4, 8, 8, 16],
        '#' => [10, 31, 10, 10, 31, 10, 0],
        '!' => [4, 4, 4, 4, 4, 0, 4],
        '"' => [10, 10, 10, 0, 0, 0, 0],
        '$' => [4, 15, 20, 14, 5, 30, 4],
        '&' => [12, 18, 20, 8, 21, 18, 13],
        '\'' => [4, 4, 8, 0, 0, 0, 0],
        '(' => [2, 4, 8, 8, 8, 4, 2],
        ')' => [8, 4, 2, 2, 2, 4, 8],
        '*' => [0, 21, 14, 31, 14, 21, 0],
        '+' => [0, 4, 4, 31, 4, 4, 0],
        ',' => [0, 0, 0, 0, 4, 4, 8],
        ';' => [0, 4, 4, 0, 4, 4, 8],
        '<' => [2, 4, 8, 16, 8, 4, 2],
        '=' => [0, 0, 31, 0, 31, 0, 0],
        '>' => [8, 4, 2, 1, 2, 4, 8],
        '?' => [14, 17, 1, 6, 4, 0, 4],
        '@' => [14, 17, 23, 21, 23, 16, 14],
        '[' => [14, 8, 8, 8, 8, 8, 14],
        '\\' => [16, 8, 8, 4, 2, 2, 1],
        ']' => [14, 2, 2, 2, 2, 2, 14],
        '^' => [4, 10, 17, 0, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 31],
        '`' => [8, 4, 2, 0, 0, 0, 0],
        '{' => [3, 4, 4, 8, 4, 4, 3],
        '|' => [4, 4, 4, 4, 4, 4, 4],
        '}' => [24, 4, 4, 2, 4, 4, 24],
        '~' => [0, 0, 9, 22, 0, 0, 0],
        ' ' => [0; 7],
        _ => [31, 17, 1, 6, 4, 0, 4],
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use anyhow::Context;
    use gpiocdev::{
        Request,
        line::{Bias, Value},
    };
    use std::{
        fs::{File, OpenOptions},
        io::Write,
        net::{SocketAddr, UdpSocket},
        os::fd::AsRawFd,
        sync::{atomic::Ordering, mpsc},
        thread,
        time::Instant,
    };

    const DC: u32 = 9;
    const BACKLIGHT: u32 = 12;
    const SPI_PATH: &str = "/dev/spidev0.0";
    const SPI_IOC_WR_MODE: libc::c_ulong = 0x40016b01;
    const SPI_IOC_WR_BITS_PER_WORD: libc::c_ulong = 0x40016b03;
    const SPI_IOC_WR_MAX_SPEED_HZ: libc::c_ulong = 0x40046b04;

    fn current_ipv4() -> Option<Ipv4Addr> {
        // Connecting a UDP socket selects the default route without sending a packet.
        // This does not depend on DNS or on the MQTT broker being reachable.
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
        socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
        match socket.local_addr().ok()? {
            SocketAddr::V4(address) if !address.ip().is_unspecified() => Some(*address.ip()),
            _ => None,
        }
    }

    struct Display {
        spi: File,
        outputs: Request,
    }

    impl Display {
        fn open(gpio_chip: &Path) -> Result<Self> {
            let outputs = Request::builder()
                .on_chip(gpio_chip.to_path_buf())
                .with_lines(&[DC, BACKLIGHT])
                .as_output(Value::Inactive)
                .with_consumer("growhat-display")
                .request()
                .context("request display GPIO 9/12")?;
            let spi = OpenOptions::new()
                .write(true)
                .open(SPI_PATH)
                .context("open /dev/spidev0.0")?;
            let fd = spi.as_raw_fd();
            let mode: u8 = 0;
            let bits: u8 = 8;
            let speed: u32 = 4_000_000;
            // SAFETY: the pointers are valid for the synchronous spidev ioctl calls.
            for (name, request, pointer) in [
                (
                    "mode",
                    SPI_IOC_WR_MODE,
                    &mode as *const u8 as *const libc::c_void,
                ),
                (
                    "bits",
                    SPI_IOC_WR_BITS_PER_WORD,
                    &bits as *const u8 as *const libc::c_void,
                ),
                (
                    "speed",
                    SPI_IOC_WR_MAX_SPEED_HZ,
                    &speed as *const u32 as *const libc::c_void,
                ),
            ] {
                if unsafe { libc::ioctl(fd, request, pointer) } < 0 {
                    return Err(std::io::Error::last_os_error())
                        .with_context(|| format!("set SPI {name}"));
                }
            }
            let mut display = Self { spi, outputs };
            display.init()?;
            display.outputs.set_value(BACKLIGHT, Value::Active)?;
            Ok(display)
        }
        fn send(&mut self, is_data: bool, bytes: &[u8]) -> Result<()> {
            self.outputs.set_value(
                DC,
                if is_data {
                    Value::Active
                } else {
                    Value::Inactive
                },
            )?;
            for chunk in bytes.chunks(4096) {
                self.spi.write_all(chunk)?;
            }
            Ok(())
        }
        fn cmd(&mut self, cmd: u8, data: &[u8]) -> Result<()> {
            self.send(false, &[cmd])?;
            if !data.is_empty() {
                self.send(true, data)?;
            }
            Ok(())
        }
        fn init(&mut self) -> Result<()> {
            self.cmd(0x01, &[])?;
            thread::sleep(Duration::from_millis(150));
            self.cmd(0x11, &[])?;
            thread::sleep(Duration::from_millis(500));
            for (cmd, data) in [
                (0xb1, &[1, 0x2c, 0x2d][..]),
                (0xb2, &[1, 0x2c, 0x2d]),
                (0xb3, &[1, 0x2c, 0x2d, 1, 0x2c, 0x2d]),
                (0xb4, &[7]),
                (0xc0, &[0xa2, 2, 0x84]),
                (0xc1, &[0x0a, 0]),
                (0xc3, &[0x8a, 0x2a]),
                (0xc4, &[0x8a, 0xee]),
                (0xc5, &[0x0e]),
                (0x21, &[]),
                (0x36, &[0xc8]),
                (0x3a, &[5]),
                (0x2a, &[0, 26, 0, 105]),
                (0x2b, &[0, 1, 0, 160]),
                (
                    0xe0,
                    &[2, 28, 7, 18, 55, 50, 41, 45, 41, 37, 43, 57, 0, 1, 3, 16],
                ),
                (
                    0xe1,
                    &[3, 29, 7, 6, 46, 44, 41, 45, 46, 46, 55, 63, 0, 0, 2, 16],
                ),
            ] {
                self.cmd(cmd, data)?;
            }
            self.cmd(0x13, &[])?;
            thread::sleep(Duration::from_millis(100));
            self.cmd(0x29, &[])?;
            thread::sleep(Duration::from_millis(100));
            Ok(())
        }
        fn draw(&mut self, pixels: &[u16]) -> Result<()> {
            if pixels.len() != WIDTH * HEIGHT {
                bail!("invalid frame size");
            }
            self.cmd(0x2a, &[0, 26, 0, 105])?;
            self.cmd(0x2b, &[0, 1, 0, 160])?;
            self.send(false, &[0x2c])?;
            self.outputs.set_value(DC, Value::Active)?;
            let data = rotated_rgb565_bytes(pixels);
            for chunk in data.chunks(4096) {
                self.spi.write_all(chunk)?;
            }
            Ok(())
        }
    }

    impl Drop for Display {
        fn drop(&mut self) {
            let _ = self.outputs.set_value(BACKLIGHT, Value::Inactive);
        }
    }

    pub fn start(gpio_chip: &Path, stop: Arc<AtomicBool>) -> Result<Panel> {
        let mut display = Display::open(gpio_chip)?;
        let inputs = Request::builder()
            .on_chip(gpio_chip.to_path_buf())
            .with_lines(&BUTTONS.map(|(_, pin)| pin))
            .as_input()
            .with_bias(Bias::PullUp)
            .with_consumer("growhat-buttons")
            .request()
            .context("request button GPIO 5/6/16/24")?;
        let mut state = ScreenState {
            ip_address: current_ipv4(),
            ..ScreenState::default()
        };
        // A held button at startup is reported as held with count zero; only a
        // subsequent press transition increments its count.
        let mut levels = read_button_levels(&inputs)?;
        let mut stable_levels = None;
        for _ in 0..10 {
            thread::sleep(DEBOUNCE);
            let next = read_button_levels(&inputs)?;
            if next == levels {
                stable_levels = Some(next);
                break;
            }
            levels = next;
        }
        let stable_levels = stable_levels.context("button inputs did not settle during startup")?;
        for (index, pressed) in stable_levels.into_iter().enumerate() {
            state.buttons[index].raw = pressed;
            state.buttons[index].stable = pressed;
        }
        let mut last_frame = render(&state);
        display
            .draw(&last_frame)
            .context("draw initial panel frame")?;
        let (commands, command_rx) = mpsc::sync_channel(16);
        let (event_tx, events) = tokio::sync::mpsc::channel(64);
        for event in button_snapshots(&state.buttons) {
            let _ = event_tx.try_send(event);
        }
        let worker = thread::Builder::new()
            .name("growhat-panel".into())
            .spawn(move || -> Result<()> {
                let started = Instant::now();
                let mut state = state;
                let mut message_until = None;
                let mut last_snapshot = Instant::now();
                let mut last_ip_check = Instant::now();
                while !stop.load(Ordering::Relaxed) {
                    let mut dirty = false;
                    match command_rx.recv_timeout(Duration::from_millis(10)) {
                        Ok(PanelCommand::Readings(readings)) => {
                            state.readings = readings;
                            state.sample_number = state.sample_number.wrapping_add(1);
                            dirty = true;
                        }
                        Ok(PanelCommand::Message { text, ttl_seconds }) => {
                            state.message = Some(text);
                            message_until =
                                Some(Instant::now() + Duration::from_secs(ttl_seconds.min(3600)));
                            dirty = true;
                        }
                        Ok(PanelCommand::VirtualPress { button }) => {
                            if let Some(event) = virtual_press(&mut state.buttons, &button) {
                                let _ = event_tx.try_send(event);
                                state.message = Some(format!("REMOTE {button}"));
                                message_until = Some(Instant::now() + Duration::from_secs(1));
                                eprintln!("BUTTON virtual press {button}");
                                dirty = true;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    for (index, (label, pin)) in BUTTONS.iter().enumerate() {
                        let pressed = inputs
                            .value(*pin)
                            .with_context(|| format!("read button {label}"))?
                            == Value::Inactive;
                        if let Some(event) =
                            state.buttons[index].observe(label, pressed, started.elapsed())
                        {
                            let _ = event_tx.try_send(event);
                            dirty = true;
                        }
                    }
                    if message_until.is_some_and(|until| Instant::now() >= until) {
                        state.message = None;
                        message_until = None;
                        dirty = true;
                    }
                    if last_snapshot.elapsed() >= Duration::from_secs(1) {
                        for event in button_snapshots(&state.buttons) {
                            let _ = event_tx.try_send(event);
                        }
                        last_snapshot = Instant::now();
                    }
                    if last_ip_check.elapsed() >= Duration::from_secs(5) {
                        let address = current_ipv4();
                        if address != state.ip_address {
                            eprintln!(
                                "DISPLAY IP changed to {}",
                                address
                                    .map_or_else(|| "unavailable".to_owned(), |ip| ip.to_string())
                            );
                            state.ip_address = address;
                            dirty = true;
                        }
                        last_ip_check = Instant::now();
                    }
                    if dirty {
                        let frame = render(&state);
                        if frame != last_frame {
                            display.draw(&frame)?;
                            last_frame = frame;
                            if let Some(text) = &state.message {
                                eprintln!("DISPLAY rendered text={text}");
                            } else {
                                eprintln!("DISPLAY rendered sample={}", state.sample_number);
                            }
                        }
                    }
                }
                Ok(())
            })
            .context("spawn panel worker")?;
        Ok(Panel {
            commands,
            events,
            worker,
        })
    }

    fn read_button_levels(inputs: &Request) -> Result<[bool; 4]> {
        let mut levels = [false; 4];
        for (index, (label, pin)) in BUTTONS.iter().enumerate() {
            levels[index] = inputs
                .value(*pin)
                .with_context(|| format!("read button {label}"))?
                == Value::Inactive;
        }
        Ok(levels)
    }
}

#[cfg(target_os = "linux")]
pub use linux::start;

#[cfg(not(target_os = "linux"))]
pub fn start(_gpio_chip: &Path, _stop: Arc<AtomicBool>) -> Result<Panel> {
    bail!("Grow HAT panel is supported only on Linux")
}

impl Panel {
    pub fn start(gpio_chip: &Path, stop: Arc<AtomicBool>) -> Result<Self> {
        start(gpio_chip, stop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensor::ReadingStatus;
    #[test]
    fn debounce_counts_presses_once_and_reports_release() {
        let mut button = ButtonState::default();
        assert_eq!(button.observe("A", true, Duration::ZERO), None);
        assert_eq!(button.observe("A", true, Duration::from_millis(29)), None);
        assert_eq!(
            button
                .observe("A", true, Duration::from_millis(30))
                .unwrap()
                .count,
            1
        );
        assert_eq!(button.observe("A", true, Duration::from_secs(1)), None);
        assert_eq!(button.observe("A", false, Duration::from_secs(2)), None);
        let released = button
            .observe("A", false, Duration::from_millis(2030))
            .unwrap();
        assert!(!released.pressed);
        assert_eq!(released.count, 1);
    }
    #[test]
    fn startup_snapshot_reports_held_without_counting_it() {
        let mut buttons = std::array::from_fn(|_| ButtonState::default());
        buttons[0].raw = true;
        buttons[0].stable = true;
        let events = button_snapshots(&buttons);
        assert_eq!(
            events[0],
            ButtonEvent {
                button: "A".into(),
                pressed: true,
                count: 0
            }
        );
        assert_eq!(
            events[1],
            ButtonEvent {
                button: "B".into(),
                pressed: false,
                count: 0
            }
        );
        assert_eq!(events[2].button, "X");
        assert_eq!(events[3].button, "Y");
    }
    #[test]
    fn virtual_press_increments_count_without_faking_a_physical_hold() {
        let mut buttons = std::array::from_fn(|_| ButtonState::default());
        assert_eq!(virtual_press(&mut buttons, "A").unwrap().count, 1);
        assert_eq!(virtual_press(&mut buttons, "A").unwrap().count, 2);
        assert!(!virtual_press(&mut buttons, "A").unwrap().pressed);
        assert!(virtual_press(&mut buttons, "Q").is_none());
    }
    #[test]
    fn renderer_uses_message_and_reading_status() {
        let reading = SensorReading {
            channel: 1,
            raw_hz: Some(10.0),
            moisture_percent: Some(50.0),
            status: ReadingStatus::Valid,
            age_ms: Some(0),
            error: None,
        };
        let mut state = ScreenState {
            message: Some("Test display".into()),
            readings: vec![reading],
            ..ScreenState::default()
        };
        assert_eq!(
            layout_message(state.message.as_deref().unwrap()).0[0],
            "Test display"
        );
        assert_eq!(display_lines(&state)[1], "M1: 50%  10.0Hz");
        assert_eq!(display_lines(&state)[0], "IP unavailable #0000");
        state.ip_address = Some(Ipv4Addr::new(192, 168, 1, 216));
        state.sample_number = 1;
        assert_eq!(display_lines(&state)[0], "IP 192.168.1.216 #0001");
        state.ip_address = Some(Ipv4Addr::new(255, 255, 255, 255));
        assert!(display_lines(&state)[0].len() <= 25);
        assert_eq!(render(&state).len(), WIDTH * HEIGHT);
        state.readings[0].status = ReadingStatus::Stale;
        assert_eq!(display_lines(&state)[1], "M1: STALE");
        state.readings[0].status = ReadingStatus::NoSignal;
        assert_eq!(display_lines(&state)[1], "M1: NO SIGNAL");
    }
    #[test]
    fn message_wraps_into_five_lines_and_marks_overflow() {
        let exact = "A".repeat(120);
        assert!(message_fits(&exact));
        let (lines, overflow) = layout_message(&exact);
        assert!(!overflow);
        assert_eq!(lines.len(), 5);
        assert!(lines.iter().all(|line| line.len() == 24));
        assert_eq!(layout_message("top\nsecond").0, vec!["top", "second"]);
        assert!(!message_fits(&format!("{exact}X")));
        assert!(layout_message(&format!("{exact}X")).0[4].ends_with("..."));
    }
    #[test]
    fn rotation_matches_vendor_clockwise_layout() {
        let mut pixels = vec![0; WIDTH * HEIGHT];
        pixels[0] = 0x1234;
        pixels[79 * WIDTH + 159] = 0xabcd;
        let bytes = rotated_rgb565_bytes(&pixels);
        assert_eq!(bytes.len(), WIDTH * HEIGHT * 2);
        assert_eq!(&bytes[79 * 2..79 * 2 + 2], &[0x12, 0x34]);
        assert_eq!(
            &bytes[(WIDTH * HEIGHT - 80) * 2..(WIDTH * HEIGHT - 80) * 2 + 2],
            &[0xab, 0xcd]
        );
    }
    #[test]
    fn printable_ascii_has_drawable_glyphs() {
        let fallback = glyph('\0');
        for byte in 32u8..=126 {
            let ch = (byte as char).to_ascii_uppercase();
            assert_ne!(glyph(ch), fallback, "missing glyph for {ch}");
        }
    }
}
