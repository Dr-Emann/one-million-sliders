use std::{
    io::{self, BufWriter, Write},
    path::Path,
    time::{Duration, Instant},
};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Message {
    SetByte { offset: u32, value: u8 },
    Toggle { offset: u32 },
}

impl Message {
    fn to_record(self) -> [u8; 5] {
        const TYPE_MASK: u32 = 1 << 31;
        let (offset, value) = match self {
            Message::SetByte { offset, value } => {
                debug_assert_eq!(offset & TYPE_MASK, 0);
                (offset, value)
            }
            Message::Toggle { offset } => {
                debug_assert_eq!(offset & TYPE_MASK, 0);
                (offset | TYPE_MASK, 0)
            }
        };
        let mut result = [0; 5];
        result[..4].copy_from_slice(&offset.to_le_bytes());
        result[4] = value;
        result
    }
}

pub struct Log {
    tx: std::sync::mpsc::SyncSender<Message>,
}

impl Log {
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::_new(path.as_ref())
    }

    fn _new(path: &Path) -> io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        let (tx, rx) = std::sync::mpsc::sync_channel(100);
        std::thread::spawn(move || {
            let mut file = BufWriter::new(file);
            let mut next_flush: Option<Instant> = None;
            loop {
                let msg = if let Some(next_flush) = next_flush {
                    match rx.recv_timeout(next_flush.duration_since(Instant::now())) {
                        Ok(msg) => Some(msg),
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                } else {
                    match rx.recv() {
                        Ok(msg) => Some(msg),
                        Err(_) => break,
                    }
                };
                match msg {
                    Some(msg) => {
                        _ = handle(&mut file, msg);
                        next_flush = Some(Instant::now() + Duration::from_secs(1));
                    }
                    None => {
                        _ = file.flush();
                        next_flush = None;
                    }
                }
            }
        });

        Ok(Self { tx })
    }

    pub fn log_msg(&self, msg: Message) {
        self.tx.send(msg).unwrap();
    }
}

fn handle<W: io::Write>(mut file: W, msg: Message) -> io::Result<()> {
    let record = msg.to_record();
    file.write_all(&record)
}
