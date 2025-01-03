use crate::{flash, version, Mode};
use defmt::info;
use embassy_time::{Duration, Timer};
use embedded_io_async::Write;

pub async fn handle_command(command: &[u8], response: &mut impl Write) {
    if command.is_empty() {
        response.write_all(b"?").await.unwrap();
    } else {
        let (command, args) = command.split_first().unwrap();
        match command {
            b'D' => set_default_mode(args, response).await,
            b'E' => enumerate(args, response).await,
            b'L' => set_colors(args, response).await,
            b'M' => map_panels(args, response).await,
            b'R' => reset_all(args, response).await,
            b'V' => response
                .write_all(version::VERSION.as_bytes())
                .await
                .unwrap(),
            b'_' => test_message(args, response).await,
            _ => response.write_all(b"?").await.unwrap(),
        }
    }
    response.write_all(b"\r\n").await.unwrap();
}

async fn set_default_mode(args: &[u8], response: &mut impl Write) {
    info!("set_default_mode {}", core::str::from_utf8(args).unwrap());
    let mode = match args[0] {
        b'M' => Mode::Master,
        b'P' => Mode::Panel,
        _ => return response.write_all(b"?").await.unwrap(),
    };
    flash::set_default_mode(mode);
    response.write_all(b"OK").await.unwrap();
    Timer::after(Duration::from_millis(100)).await;
    cortex_m::peripheral::SCB::sys_reset();
}

async fn enumerate(args: &[u8], response: &mut impl Write) {
    info!("enumerate");
    response.write_all(b"E").await.unwrap();
}

async fn set_colors(args: &[u8], response: &mut impl Write) {
    info!("set_colors");
    response.write_all(b"L").await.unwrap();
}

async fn map_panels(args: &[u8], response: &mut impl Write) {
    info!("map_panels");
    response.write_all(b"M").await.unwrap();
}

async fn reset_all(args: &[u8], response: &mut impl Write) {
    info!("reset_all");
    response.write_all(b"R").await.unwrap();
}

async fn test_message(args: &[u8], response: &mut impl Write) {
    info!("test_message");
    response.write_all(b"_").await.unwrap();
}
