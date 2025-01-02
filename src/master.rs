use embedded_io_async::Write;

pub async fn handle_command(command: &[u8], response: &mut impl Write) {
    response.write_all(command).await.unwrap();
}
