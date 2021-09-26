use crate::{ErrorKind, Result};
use crate::capped_reader::{CappedReader, CappedRead};
use crate::communication::{ReadMessage, WriteMessage};

use std::io::prelude::Write;
use std::io::Read;

use std::net::TcpStream;

use serde_json::{Deserializer, Value};

pub struct JsonReader<R> {
    stream: CappedReader<R>,
}

impl<R: Read> JsonReader<R> {
    pub fn new(capped_reader: CappedReader<R>) -> JsonReader<R> {
        JsonReader {
            stream: capped_reader,
        }
    }
}

impl<'a, R: Read> ReadMessage<Value> for JsonReader<R> {
    fn read(&mut self) -> Result<Value> {
        let mut iterator = Deserializer::from_reader(&mut self.stream).into_iter::<Value>();

        match iterator.next() {
            Some(Ok(it)) => {
                self.stream.clear();
                Ok(it)
            }
            Some(Err(serde)) => {
                Err(serde.into())
            }
            _ => {
                Err(ErrorKind::NothingToRead.into())
            }
        }
    }
}

pub struct JsonWriter {
    pub stream: TcpStream,
}

impl JsonWriter {
    pub fn new(stream: TcpStream) -> JsonWriter {
        JsonWriter {
            stream: stream,
        }
    }
}

impl WriteMessage<&Value> for JsonWriter {
    fn write(&mut self, message: &Value) -> Result<()> {
        self.stream.write(message.to_string().as_bytes())?;
        self.stream.flush()?;
        Ok(())
    }
}
