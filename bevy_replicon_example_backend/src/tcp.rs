use std::{
    error::Error,
    io::{self, IoSlice, Read, Write},
    net::TcpStream,
    slice,
};

pub(super) fn read_message(stream: &mut TcpStream) -> io::Result<(u8, Vec<u8>)> {
    let mut channel_id = 0;
    stream.read_exact(slice::from_mut(&mut channel_id))?;

    let mut size_bytes = [0; 2];
    stream.read_exact(&mut size_bytes)?;
    let message_size = u16::from_le_bytes(size_bytes);

    let mut message = vec![0; message_size as usize];
    stream.read_exact(&mut message)?;

    Ok((channel_id, message))
}

pub(super) fn send_message(
    stream: &mut TcpStream,
    channel_id: u8,
    message: &[u8],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let message_size: u16 = message.len().try_into()?;
    let channel_id = &[channel_id];
    let message_size = &message_size.to_le_bytes();
    let packet = [
        IoSlice::new(channel_id),
        IoSlice::new(message_size),
        IoSlice::new(&message),
    ];

    // Write as a single message to avoid splitting between packets.
    stream.write_vectored(&packet)?;

    Ok(())
}
