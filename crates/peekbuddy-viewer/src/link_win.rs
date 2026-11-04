//! The engine link over the real named pipe.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use peekbuddy_core::ipc::Message;
use peekbuddy_win::pipe::PipeClient;

use crate::link::{Connection, Connector};

/// Connects to the engine's pipe, checking that the server runs this exe.
#[derive(Debug)]
pub struct PipeConnector {
    /// The pipe name.
    pub name: String,
    /// The exe the engine must be running (ours: one exe, two roles).
    pub exe: PathBuf,
}

/// A connected pipe.
#[derive(Debug)]
pub struct PipeConnection(PipeClient);

impl Connector for PipeConnector {
    type Conn = PipeConnection;

    fn connect(&mut self) -> io::Result<PipeConnection> {
        // No waiting: the link thread retries on its own schedule.
        PipeClient::connect(&self.name, Some(&self.exe), Duration::ZERO).map(PipeConnection)
    }
}

impl Connection for PipeConnection {
    fn send(&mut self, message: &Message) -> io::Result<()> {
        self.0.send(message)
    }

    fn receive(&mut self) -> io::Result<Option<Message>> {
        self.0.receive()
    }

    fn available(&self) -> io::Result<u32> {
        self.0.available()
    }
}
