# Telephony — AT Commands

> Controlling the cellular modem directly via serial commands.

---

## Overview

Android normally handles telephony through a complex stack:

```
App → TelephonyManager → RIL (Radio Interface Layer) → rild daemon → AT commands → Modem
```

Peko Agent **goes directly to the modem** via serial device:

```
Peko Agent → /dev/ttyACM0 → Modem
```

## Serial Connection Setup

### Finding the Modem

```rust
// Scan /sys/class/tty/ for serial devices
for entry in read_dir("/sys/class/tty")? {
    let name = entry.file_name();
    // Common modem device names:
    // ttyACM0 (USB modems)
    // ttyUSB0, ttyUSB2 (some Qualcomm)
    // ttyHS0, ttyMSM0 (Qualcomm UART)
    if name.starts_with("ttyACM") || name.starts_with("ttyUSB") {
        // Verify it's a modem by sending AT and checking for OK
        if probe_modem(&format!("/dev/{}", name))? {
            return Ok(path);
        }
    }
}
```

### Serial Configuration

```rust
use nix::sys::termios::*;

let fd = open("/dev/ttyACM0", O_RDWR | O_NOCTTY)?;
let mut termios = tcgetattr(fd)?;

// 115200 baud, 8 data bits, no parity, 1 stop bit (8N1)
cfsetspeed(&mut termios, BaudRate::B115200)?;
termios.control_flags |= CS8;
termios.control_flags &= !(PARENB | CSTOPB | CRTSCTS);
termios.control_flags |= CLOCAL | CREAD;

// Raw mode (no echo, no canonical processing)
cfmakeraw(&mut termios);

// Timeouts
termios.control_chars[VTIME] = 10;  // 1 second timeout
termios.control_chars[VMIN] = 0;    // Non-blocking

tcsetattr(fd, SetArg::TCSANOW, &termios)?;
```

## Core AT Commands

### Basic

| Command | Response | Purpose |
|---|---|---|
| `AT` | `OK` | Test modem is alive |
| `AT+CPIN?` | `+CPIN: READY` | Check SIM status |
| `AT+CSQ` | `+CSQ: 18,99` | Signal strength (0-31) |
| `AT+COPS?` | `+COPS: 0,0,"Carrier"` | Current network operator |

### SMS

```
# Set text mode (vs PDU mode)
AT+CMGF=1
OK

# Send SMS
AT+CMGS="+1234567890"
> Hello from Peko<Ctrl-Z>
+CMGS: 42
OK

# Read SMS messages
AT+CMGL="ALL"
+CMGL: 0,"REC READ","+1234567890","","24/03/15,10:30:00+00"
Message text here
OK

# Delete SMS
AT+CMGD=0
OK
```

### Voice Calls

```
# Dial a number (semicolon = voice call)
ATD+1234567890;
OK

# Answer incoming call
ATA
OK

# Hang up
ATH
OK

# Reject incoming call
ATH
OK
```

### Unsolicited Result Codes (URCs)

The modem sends these without being asked:

| URC | Meaning |
|---|---|
| `RING` | Incoming call |
| `+CLIP: "+1234567890"` | Caller ID (if enabled) |
| `NO CARRIER` | Call ended |
| `BUSY` | Called party busy |
| `+CMTI: "SM",0` | New SMS received |
| `+CREG: 1` | Network registration changed |

The [[../implementation/peko-hal|SerialModem]] must handle these asynchronously while waiting for command responses.

## Command/Response Protocol

```rust
impl SerialModem {
    pub fn send_command(&self, cmd: &str, timeout_ms: u64) -> Result<String> {
        // Send command with \r terminator
        write(self.fd, format!("{}\r", cmd).as_bytes())?;

        // Read response lines until OK, ERROR, or timeout
        let mut response = String::new();
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);

        loop {
            let line = self.read_line(deadline)?;
            if line == "OK" {
                return Ok(response);
            } else if line.starts_with("ERROR") || line.starts_with("+CME ERROR") {
                return Err(ModemError::CommandFailed(line));
            } else {
                response.push_str(&line);
                response.push('\n');
            }
        }
    }
}
```

## SMS Send Flow (Complete)

```
1. SmsTool::execute({"to": "+1234567890", "message": "Hello"})
2.   → modem.send_command("AT+CMGF=1")        // Text mode
3.   → modem.send_command("AT+CMGS=\"+1234567890\"")  // Start message
4.   → (modem responds with "> " prompt)
5.   → write(fd, "Hello\x1A")                  // Message + Ctrl-Z
6.   → (modem responds with "+CMGS: N\r\nOK")
7.   → ToolResult { content: "SMS sent (ref: N)", is_error: false }
```

## Device-Specific Quirks

| Chipset | Modem device | Notes |
|---|---|---|
| Qualcomm (SDM/SM) | `/dev/smd7` or `/dev/ttyMSM0` | May use QMI instead of AT |
| MediaTek | `/dev/ttyACM0` | Standard AT |
| Samsung Exynos | `/dev/ttyACM0` | Standard AT |
| USB modems | `/dev/ttyUSB0`-`/dev/ttyUSB3` | AT port varies |

Some modern chipsets use QMI (Qualcomm MSM Interface) instead of AT commands. QMI is binary protocol — more complex but more capable. For MVP, AT commands cover most devices.

## Related

- [[../implementation/peko-hal]] — SerialModem struct
- [[../implementation/peko-tools-android]] — SmsTool, CallTool
- [[Linux-Kernel-Interfaces]] — Serial subsystem overview
- [[SELinux-Policy]] — Permission for tty devices
- [[../roadmap/Challenges-And-Risks]] — Modem compatibility challenges

---

#knowledge #telephony #at-commands #modem #sms
