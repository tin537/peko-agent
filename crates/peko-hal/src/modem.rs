use std::fs::{self, OpenOptions, File};
use std::io::{Read, Write, BufRead, BufReader};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct SerialModem {
    file: File,
    device_path: PathBuf,
}

impl SerialModem {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;

        Self::configure_termios(file.as_raw_fd())?;

        let mut modem = Self {
            file,
            device_path: path.to_path_buf(),
        };

        // verify modem responds
        let response = modem.send_command("AT", 2000)?;
        if !response.contains("OK") {
            anyhow::bail!("modem at {} did not respond to AT", path.display());
        }

        Ok(modem)
    }

    pub fn find_and_open() -> anyhow::Result<Self> {
        let candidates = [
            // Real device modem paths
            "/dev/ttyACM0",
            "/dev/ttyACM1",
            "/dev/ttyUSB0",
            "/dev/ttyUSB2",
            "/dev/ttyMSM0",
            // Emulator (goldfish/ranchu) modem paths
            "/dev/ttyGF0",
            "/dev/ttyGF1",
            "/dev/ttyS0",
            "/dev/ttyS1",
        ];

        for path_str in &candidates {
            let path = Path::new(path_str);
            if path.exists() {
                if let Ok(modem) = Self::open(path) {
                    return Ok(modem);
                }
            }
        }

        anyhow::bail!("no modem device found")
    }

    fn configure_termios(fd: i32) -> anyhow::Result<()> {
        unsafe {
            let mut termios: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut termios) != 0 {
                anyhow::bail!("tcgetattr failed");
            }

            libc::cfmakeraw(&mut termios);
            libc::cfsetispeed(&mut termios, libc::B115200);
            libc::cfsetospeed(&mut termios, libc::B115200);

            termios.c_cflag |= libc::CS8 | libc::CLOCAL | libc::CREAD;
            termios.c_cflag &= !(libc::PARENB | libc::CSTOPB | libc::CRTSCTS);

            termios.c_cc[libc::VTIME] = 10; // 1 second timeout
            termios.c_cc[libc::VMIN] = 0;

            if libc::tcsetattr(fd, libc::TCSANOW, &termios) != 0 {
                anyhow::bail!("tcsetattr failed");
            }
        }
        Ok(())
    }

    pub fn send_command(&mut self, cmd: &str, timeout_ms: u64) -> anyhow::Result<String> {
        let cmd_with_cr = format!("{}\r", cmd);
        self.file.write_all(cmd_with_cr.as_bytes())?;
        self.file.flush()?;

        let mut response = String::new();
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut buf = [0u8; 256];

        loop {
            if Instant::now() > deadline {
                anyhow::bail!("timeout waiting for modem response to '{}'", cmd);
            }

            match self.file.read(&mut buf) {
                Ok(0) => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Ok(n) => {
                    response.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if response.contains("OK") || response.contains("ERROR")
                        || response.contains("+CME ERROR")
                    {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
        }

        Ok(response)
    }

    pub fn send_sms(&mut self, to: &str, message: &str) -> anyhow::Result<String> {
        self.send_command("AT+CMGF=1", 2000)?;

        let cmd = format!("AT+CMGS=\"{}\"", to);
        let cmd_with_cr = format!("{}\r", cmd);
        self.file.write_all(cmd_with_cr.as_bytes())?;
        self.file.flush()?;

        std::thread::sleep(Duration::from_millis(500));

        // Send message body + Ctrl-Z
        let body = format!("{}\x1A", message);
        self.file.write_all(body.as_bytes())?;
        self.file.flush()?;

        // Wait for response
        let mut response = String::new();
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut buf = [0u8; 256];

        loop {
            if Instant::now() > deadline {
                anyhow::bail!("timeout waiting for SMS send confirmation");
            }
            match self.file.read(&mut buf) {
                Ok(0) => { std::thread::sleep(Duration::from_millis(50)); }
                Ok(n) => {
                    response.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if response.contains("OK") || response.contains("ERROR") {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(e.into()),
            }
        }

        Ok(response)
    }

    pub fn dial(&mut self, number: &str) -> anyhow::Result<String> {
        self.send_command(&format!("ATD{};", number), 5000)
    }

    pub fn hangup(&mut self) -> anyhow::Result<String> {
        self.send_command("ATH", 5000)
    }

    pub fn answer(&mut self) -> anyhow::Result<String> {
        self.send_command("ATA", 5000)
    }

    pub fn device_path(&self) -> &Path {
        &self.device_path
    }
}
