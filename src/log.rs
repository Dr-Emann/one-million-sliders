use std::{
    io::{self, BufWriter, Write},
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Message {
    SetByte {
        time: SystemTime,
        offset: u32,
        value: u8,
    },
    Toggle {
        time: SystemTime,
        offset: u32,
    },
}

const RECORD_SIZE: usize = size_of::<u128>() + size_of::<u32>() + size_of::<u8>();

impl Message {
    fn to_record(self) -> [u8; RECORD_SIZE] {
        const TYPE_MASK: u32 = 1 << 31;
        let (time, offset, value) = match self {
            Message::SetByte {
                time,
                offset,
                value,
            } => {
                debug_assert_eq!(offset & TYPE_MASK, 0);
                (time, offset, value)
            }
            Message::Toggle { time, offset } => {
                debug_assert_eq!(offset & TYPE_MASK, 0);
                (time, offset | TYPE_MASK, 0)
            }
        };
        let time_diff = time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let mut result = [0; RECORD_SIZE];
        result[0..16].copy_from_slice(&time_diff.to_le_bytes());
        result[16..20].copy_from_slice(&offset.to_le_bytes());
        result[20] = value;
        assert_eq!(20, RECORD_SIZE - 1);
        result
    }
}

pub struct Log {
    tx: std::sync::mpsc::SyncSender<Message>,
    join_handle: std::thread::JoinHandle<()>,
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
        let join_handle = std::thread::spawn(move || {
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

        Ok(Self { tx, join_handle })
    }

    pub fn log_msg(&self, msg: Message) {
        self.tx.send(msg).unwrap();
    }

    pub fn finish(self) {
        drop(self.tx);
        self.join_handle.join().unwrap();
    }
}

fn handle<W: io::Write>(mut file: W, msg: Message) -> io::Result<()> {
    let record = msg.to_record();
    file.write_all(&record)
}
