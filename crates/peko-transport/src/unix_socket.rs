//! UnixSocketProvider — connects to peko-llm-daemon over a Unix Domain Socket.
//!
//! The socket path may be:
//!   - `/data/local/tmp/peko.sock`  — filesystem socket (blocked by SELinux on Android shell)
//!   - `@peko-llm`                   — Linux abstract namespace (SELinux-safe on Android)
//!
//! On non-Linux hosts (macOS dev), abstract namespace is rejected with a clear error.
//! On the wire: plain HTTP/1.1 + chunked transfer encoding + SSE frames.
//! We don't pull in hyper/hyperlocal — a focused ~250-line implementation
//! reuses `SseParser` and the OpenAI parsing from `OpenAICompatProvider`.

use crate::provider::{LlmProvider, Message};
use crate::sse::SseParser;
use crate::stream::StreamEvent;
use crate::openai_compat::OpenAICompatProvider;

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use serde_json::json;
use std::collections::HashMap;
use std::io;
use std::os::fd::FromRawFd;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Connect to a Unix socket in the Linux abstract namespace.
/// Uses libc directly so it works on both `target_os = "linux"` and `"android"`.
/// Abstract sockets are not supported on macOS/BSD — we return Unsupported there.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn connect_abstract(name: &str) -> io::Result<UnixStream> {
    unsafe {
        let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        let mut addr: libc::sockaddr_un = std::mem::zeroed();
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;

        // Abstract namespace: first byte of sun_path is NUL, followed by the name.
        let name_bytes = name.as_bytes();
        let max_name_len = addr.sun_path.len() - 1;
        if name_bytes.len() > max_name_len {
            libc::close(fd);
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("abstract socket name too long (max {})", max_name_len),
            ));
        }
        // sun_path[0] stays 0 (already zeroed), name at [1..]
        for (i, &b) in name_bytes.iter().enumerate() {
            addr.sun_path[i + 1] = b as libc::c_char;
        }
        // addrlen = offsetof(sockaddr_un, sun_path) + 1 (NUL prefix) + name length
        let addrlen = (std::mem::size_of::<libc::sa_family_t>() + 1 + name_bytes.len())
            as libc::socklen_t;

        let ret = libc::connect(
            fd,
            &addr as *const _ as *const libc::sockaddr,
            addrlen,
        );
        if ret < 0 {
            let err = io::Error::last_os_error();
            libc::close(fd);
            return Err(err);
        }

        // Set non-blocking for tokio
        let flags = libc::fcntl(fd, libc::F_GETFL, 0);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            let err = io::Error::last_os_error();
            libc::close(fd);
            return Err(err);
        }

        let std_stream = std::os::unix::net::UnixStream::from_raw_fd(fd);
        // from_std takes ownership of the fd
        UnixStream::from_std(std_stream)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn connect_abstract(_name: &str) -> io::Result<UnixStream> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "abstract socket namespace is Linux/Android only",
    ))
}

pub struct UnixSocketProvider {
    socket_path: String,
    model: String,
    max_tokens: usize,
}

impl UnixSocketProvider {
    pub fn new(socket_path: String, model: String, max_tokens: usize) -> Self {
        Self { socket_path, model, max_tokens }
    }

    /// Connect to the daemon. Supports both filesystem paths and abstract namespace
    /// (paths starting with '@' — Linux + Android only).
    async fn connect(&self) -> io::Result<UnixStream> {
        if let Some(abstract_name) = self.socket_path.strip_prefix('@') {
            connect_abstract(abstract_name)
        } else {
            UnixStream::connect(&self.socket_path).await
        }
    }
}

/// Build the OpenAI chat completions JSON body
fn build_request_body(
    system_prompt: &str,
    messages: &[Message],
    tools: &[serde_json::Value],
    model: &str,
    max_tokens: usize,
) -> serde_json::Value {
    let mut oai_messages = vec![json!({
        "role": "system",
        "content": system_prompt,
    })];
    for msg in messages {
        // Fan out: a single neutral msg may emit a tool-result message
        // plus a follow-up user message carrying the attached image.
        oai_messages.extend(OpenAICompatProvider::to_openai_message(msg));
    }

    let mut body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "stream": true,
        "messages": oai_messages,
    });

    if !tools.is_empty() {
        let oai_tools: Vec<serde_json::Value> = tools.iter().map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t["name"],
                    "description": t["description"],
                    "parameters": t["input_schema"],
                }
            })
        }).collect();
        body["tools"] = json!(oai_tools);
    }

    body
}

/// Write an HTTP/1.1 POST request to the stream.
async fn write_http_request(
    stream: &mut UnixStream,
    body_json: &[u8],
) -> io::Result<()> {
    let headers = format!(
        "POST /v1/chat/completions HTTP/1.1\r\n\
         Host: localhost\r\n\
         User-Agent: peko-agent/0.1\r\n\
         Accept: text/event-stream\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        body_json.len()
    );
    stream.write_all(headers.as_bytes()).await?;
    stream.write_all(body_json).await?;
    stream.flush().await?;
    Ok(())
}

/// Read HTTP/1.1 response headers off the stream. Returns (status_code, is_chunked).
async fn read_http_headers<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> io::Result<(u16, bool)> {
    let mut status_line = String::new();
    reader.read_line(&mut status_line).await?;

    // Parse "HTTP/1.1 200 OK"
    let status = status_line.split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("bad status line: {:?}", status_line)))?;

    let mut is_chunked = false;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        let trimmed = line.trim_end_matches("\r\n");
        if trimmed.is_empty() { break; } // end of headers

        if let Some((name, value)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case("transfer-encoding")
                && value.trim().eq_ignore_ascii_case("chunked")
            {
                is_chunked = true;
            }
        }
    }
    Ok((status, is_chunked))
}

/// Read a chunked HTTP body — returns the next body-chunk bytes (possibly empty at EOF).
/// HTTP/1.1 chunked format: `<hex-length>\r\n<data>\r\n` ... `0\r\n\r\n`
async fn read_next_chunk<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> io::Result<Option<Vec<u8>>> {
    let mut size_line = String::new();
    reader.read_line(&mut size_line).await?;
    let size_line = size_line.trim_end_matches("\r\n").trim();
    if size_line.is_empty() {
        return Ok(Some(Vec::new())); // spurious blank, caller keeps reading
    }
    // Optional chunk-extensions after ';' — ignore
    let size_hex = size_line.split(';').next().unwrap_or(size_line).trim();
    let size = usize::from_str_radix(size_hex, 16)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, format!("bad chunk size: {:?}", size_line)))?;
    if size == 0 {
        // Trailer headers (usually empty) + final \r\n
        loop {
            let mut trailer = String::new();
            reader.read_line(&mut trailer).await?;
            if trailer.trim_end_matches("\r\n").is_empty() { break; }
        }
        return Ok(None); // EOF marker
    }
    let mut buf = vec![0u8; size];
    reader.read_exact(&mut buf).await?;
    // Trailing \r\n after chunk data
    let mut crlf = [0u8; 2];
    reader.read_exact(&mut crlf).await?;
    Ok(Some(buf))
}

#[async_trait]
impl LlmProvider for UnixSocketProvider {
    async fn stream_completion(
        &self,
        system_prompt: &str,
        messages: &[Message],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<StreamEvent>>> {
        let body = build_request_body(system_prompt, messages, tools, &self.model, self.max_tokens);
        let body_bytes = serde_json::to_vec(&body)?;

        let mut stream = self.connect().await
            .map_err(|e| anyhow::anyhow!("UDS connect failed ({}): {}", self.socket_path, e))?;

        write_http_request(&mut stream, &body_bytes).await?;

        // Split the stream but keep the write half alive — dropping it would send FIN,
        // and cpp-httplib interprets half-close as client disconnect, aborting the response.
        let (read_half, write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);

        let (status, is_chunked) = read_http_headers(&mut reader).await?;
        if status >= 400 {
            // Drain remaining body for error message
            let mut err_body = String::new();
            let _ = reader.read_to_string(&mut err_body).await;
            anyhow::bail!("daemon HTTP {}: {}", status, err_body);
        }
        if !is_chunked {
            anyhow::bail!("daemon response not chunked (expected SSE)");
        }

        // Streaming body parser: read chunks → SseParser → parse_openai_delta → StreamEvents
        // `write_half` is held in the unfold state so it isn't dropped until the stream ends.
        let event_stream = stream::unfold(
            (reader, SseParser::new(), HashMap::<usize, (String, String, String)>::new(), false, write_half),
            |(mut reader, mut parser, mut tool_buffers, eof, write_half)| async move {
                if eof {
                    return None;
                }
                loop {
                    let chunk = match read_next_chunk(&mut reader).await {
                        Ok(Some(c)) if c.is_empty() => continue, // spurious blank
                        Ok(Some(c)) => c,
                        Ok(None) => {
                            return Some((
                                stream::iter(Vec::<anyhow::Result<StreamEvent>>::new()),
                                (reader, parser, tool_buffers, true, write_half),
                            ));
                        }
                        Err(e) => {
                            return Some((
                                stream::iter(vec![Err(anyhow::anyhow!("chunk read failed: {}", e))]),
                                (reader, parser, tool_buffers, true, write_half),
                            ));
                        }
                    };

                    let sse_events = parser.feed(&chunk);
                    let mut stream_events: Vec<anyhow::Result<StreamEvent>> = Vec::new();
                    for ev in sse_events {
                        let data: serde_json::Value = match serde_json::from_str(&ev.data) {
                            Ok(d) => d,
                            Err(_) => continue,
                        };
                        for se in OpenAICompatProvider::parse_openai_delta(&data, &mut tool_buffers) {
                            stream_events.push(Ok(se));
                        }
                    }

                    if !stream_events.is_empty() {
                        return Some((
                            stream::iter(stream_events),
                            (reader, parser, tool_buffers, false, write_half),
                        ));
                    }
                    // else: chunk produced no events, keep reading
                }
            },
        )
        .flatten();

        Ok(Box::pin(event_stream))
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn max_context_tokens(&self) -> usize {
        // Embedded daemon context comes from daemon side — we report a conservative default
        8192
    }
}
