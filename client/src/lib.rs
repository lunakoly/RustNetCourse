mod chars_reader;
mod connection;
mod commands;

use std::fs::{File};
use std::io::{BufRead};
use std::iter::Peekable;
use std::net::{TcpStream};

use connection::{
    ArsonClientSession,
    ClientSession,
    build_connection,
};

use shared::{Result, with_error_report};

use shared::connection::messages::{
    CommonMessage,
    ClientMessage,
    ServerMessage,
};

use shared::communication::{
    WriteMessage,
    explain_common_error,
    MessageProcessing,
};

use shared::connection::helpers::{
    send_file_non_blocking,
};

use chars_reader::{IntoCharsReader, CharsReader};
use commands::{Command, CommandProcessing};

fn handle_server_chunk(
    connection: &mut (impl ClientSession + 'static),
    data: &[u8],
    id: usize,
) -> Result<MessageProcessing> {
    let done = connection.accept_chunk(data, id)?;

    if !done {
        return Ok(MessageProcessing::Proceed);
    }

    let sharer = if let Some(that) = connection.remove_sharer(id)? {
        that
    } else {
        return Ok(MessageProcessing::Proceed)
    };

    println!("(Console) Downloaded {} and saved to {}", &sharer.name, &sharer.path);
    Ok(MessageProcessing::Proceed)
}

fn handle_server_common_message(
    connection: &mut (impl ClientSession + 'static),
    message: &CommonMessage,
) -> Result<MessageProcessing> {
    match message {
        CommonMessage::Chunk { data, id } => {
            handle_server_chunk(connection, &data, id.clone())
        }
    }
}

fn handle_server_agree_file_upload(
    connection: &mut (impl ClientSession + 'static),
    id: usize,
) -> Result<MessageProcessing> {
    let sharer = if let Some(it) = connection.remove_sharer(id)? {
        it
    } else {
        return Ok(MessageProcessing::Proceed)
    };

    send_file_non_blocking(connection, sharer)?;
    Ok(MessageProcessing::Proceed)
}

fn handle_server_decline_file_upload(
    connection: &mut (impl ClientSession + 'static),
    id: usize,
    reason: &str,
) -> Result<MessageProcessing> {
    let sharer = if let Some(it) = connection.remove_sharer(id)? {
        it
    } else {
        return Ok(MessageProcessing::Proceed)
    };

    println!("(Server) Nah, wait with your #{}. {}", &sharer.name, &reason.clone());
    Ok(MessageProcessing::Proceed)
}

fn handle_server_agree_file_download(
    connection: &mut (impl ClientSession + 'static),
    name: &str,
    size: usize,
    id: usize,
) -> Result<MessageProcessing> {
    connection.promote_sharer(&name, size, id)?;

    let response = ClientMessage::AgreeFileDownload {
        id: id,
    };

    connection.write_message(&response)?;
    Ok(MessageProcessing::Proceed)
}

fn handle_server_decline_file_download(
    connection: &mut (impl ClientSession + 'static),
    name: &str,
    reason: &str,
) -> Result<MessageProcessing> {
    connection.remove_unpromoted_sharer(&name)?;
    println!("(Server) Nah, I won't give you {}. {}", &name, &reason);
    Ok(MessageProcessing::Proceed)
}

fn handle_server_message(
    connection: &mut (impl ClientSession + 'static),
    message: &ServerMessage,
) -> Result<MessageProcessing> {
    match message {
        ServerMessage::Common { common } => {
            handle_server_common_message(connection, &common)
        }
        ServerMessage::AgreeFileUpload { id } => {
            handle_server_agree_file_upload(connection, id.clone())
        }
        ServerMessage::DeclineFileUpload { id, reason } => {
            handle_server_decline_file_upload(connection, id.clone(), &reason)
        }
        ServerMessage::AgreeFileDownload { name, size, id } => {
            handle_server_agree_file_download(connection, &name, size.clone(), id.clone())
        }
        ServerMessage::DeclineFileDownload { name, reason } => {
            handle_server_decline_file_download(connection, &name, &reason)
        }
        _ => {
            println!("{}", message);
            Ok(MessageProcessing::Proceed)
        }
    }
}

fn read_and_handle_server_message(
    connection: &mut (impl ClientSession + 'static)
) -> Result<MessageProcessing> {
    let message = match connection.read_message() {
        Ok(it) => it,
        Err(error) => {
            let explaination = explain_common_error(&error);
            println!("(Server) Error > {}", &explaination);
            return Ok(MessageProcessing::Stop)
        }
    };

    handle_server_message(connection, &message)
}

fn handle_server_messages(mut connection: impl ClientSession + 'static) -> Result<()> {
    loop {
        let result = read_and_handle_server_message(&mut connection)?;

        if let MessageProcessing::Stop = &result {
            break
        }
    }

    Ok(())
}

fn perform_text(
    connection: &mut impl ClientSession,
    text: &str,
) -> Result<CommandProcessing> {
    let message = ClientMessage::Text {
        text: text.to_owned(),
    };

    connection.write_message(&message)?;
    Ok(CommandProcessing::Proceed)
}

fn perform_rename(
    connection: &mut impl ClientSession,
    new_name: &str,
) -> Result<CommandProcessing> {
    let message = ClientMessage::Rename {
        new_name: new_name.to_owned(),
    };

    connection.write_message(&message)?;
    Ok(CommandProcessing::Proceed)
}

fn perform_upload_file(
    connection: &mut impl ClientSession,
    name: &str,
    path: &str,
) -> Result<CommandProcessing> {
    let id = connection.free_id()?;

    let file = File::open(path)?;
    let size = file.metadata()?.len() as usize;

    let request = ClientMessage::RequestFileUpload {
        name: name.to_owned(),
        size: size,
        id: id,
    };

    connection.prepare_sharer(path, file, name)?;
    connection.promote_sharer(name, size, id)?;

    connection.write_message(&request)?;
    Ok(CommandProcessing::Proceed)
}

fn perform_download_file(
    connection: &mut impl ClientSession,
    name: &str,
    path: &str,
) -> Result<CommandProcessing> {
    connection.prepare_sharer(path, File::create(path)?, name)?;

    let inner = ClientMessage::RequestFileDownload {
        name: name.to_owned(),
    };

    connection.write_message(&inner)?;
    Ok(CommandProcessing::Proceed)
}

fn match_user_command_with_connection(
    command: Command,
    connection: &mut impl ClientSession,
) -> Result<CommandProcessing> {
    match command {
        Command::Text { text } => {
            perform_text(connection, &text)
        }
        Command::Rename { new_name } => {
            perform_rename(connection, &new_name)
        }
        Command::UploadFile { name, path } => {
            perform_upload_file(connection, &name, &path)
        }
        Command::DownloadFile { name, path } => {
            perform_download_file(connection, &name, &path)
        }
        _ => {
            Ok(CommandProcessing::Proceed)
        }
    }
}

fn handle_user_command(
    connection: &mut Option<impl ClientSession>,
    reader: &mut Peekable<CharsReader>,
) -> Result<CommandProcessing> {
    match commands::parse(reader) {
        Command::End => {
            return Ok(CommandProcessing::Stop)
        }
        Command::Connect { address } => {
            let (
                reading_connection,
                writing_connection
            ) = build_connection(
                TcpStream::connect(address)?
            )?;

            std::thread::spawn(|| {
                with_error_report(|| handle_server_messages(reading_connection))
            });

            return Ok(CommandProcessing::Connect(writing_connection))
        }
        Command::Nothing => {}
        other => match connection {
            Some(it) => {
                match_user_command_with_connection(other, it)?;
            }
            None => {
                println!("(Console) Easy now! We should first establish a connection, all right? Go on, use /connect");
            }
        }
    }

    Ok(CommandProcessing::Proceed)
}

fn handle_user_commands() -> Result<()> {
    let stdin = std::io::stdin();
    let lock: &mut dyn BufRead = &mut stdin.lock();
    let mut reader = lock.chars().peekable();

    let mut connection: Option<ArsonClientSession> = None;

    loop {
        let result = handle_user_command(&mut connection, &mut reader)?;

        if let CommandProcessing::Stop = &result {
            break
        } else if let CommandProcessing::Connect(it) = result {
            connection = Some(it);
        }
    }

    if let Some(it) = &mut connection {
        it.write_message(&ClientMessage::Leave)?;
    }

    Ok(())
}

fn handle_connection() -> Result<()> {
    handle_user_commands()
}

pub fn start() {
    with_error_report(handle_connection);
}
