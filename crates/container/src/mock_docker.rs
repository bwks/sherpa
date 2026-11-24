use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Sender, channel};
use std::thread;
use std::time::Duration;

use bollard::Docker;

pub(crate) enum Reply {
    Json(&'static str, &'static str, &'static str),
    Exec(&'static str, &'static str, i64),
    Hang(&'static str),
}

pub(crate) struct MockDocker {
    handle: thread::JoinHandle<()>,
    done: Sender<()>,
}

impl MockDocker {
    pub(crate) fn finish(self) {
        self.done.send(()).unwrap();
        self.handle.join().unwrap();
    }
}

fn accept(listener: &TcpListener) -> (TcpStream, String, String) {
    let (mut stream, _) = listener.accept().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).unwrap();
    let length = head
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map_or(0, |(_, value)| value.trim().parse().unwrap());
    let mut body = vec![0; length];
    stream.read_exact(&mut body).unwrap();
    let request = head.lines().next().unwrap().to_owned();
    (stream, request, String::from_utf8(body).unwrap())
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
}

pub(crate) fn mock_docker(replies: Vec<Reply>) -> (Docker, MockDocker) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (done, finished) = channel();
    let handle = thread::spawn(move || {
        let mut unanswered = Vec::new();
        for reply in replies {
            match reply {
                Reply::Json(path, status, body) => {
                    let (mut stream, request, _) = accept(&listener);
                    assert!(request.contains(path), "unexpected request: {request}");
                    respond(&mut stream, status, body);
                }
                Reply::Hang(path) => {
                    let (stream, request, _) = accept(&listener);
                    assert!(request.contains(path), "unexpected request: {request}");
                    unanswered.push(stream);
                }
                Reply::Exec(command, output, exit) => {
                    let (mut stream, request, created) = accept(&listener);
                    assert!(
                        request.contains("/exec ") && created.contains(command),
                        "unexpected request: {request} {created}"
                    );
                    respond(&mut stream, "201 Created", r#"{"Id":"exec-id"}"#);
                    let (mut stream, request, _) = accept(&listener);
                    assert!(
                        request.contains("/exec/exec-id/start"),
                        "unexpected request: {request}"
                    );
                    stream
                        .write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: tcp\r\n\r\n")
                        .unwrap();
                    if created.contains(r#""AttachStdin":true"#) {
                        stream.read_to_end(&mut Vec::new()).unwrap();
                    }
                    if !output.is_empty() {
                        let mut frame = vec![1, 0, 0, 0];
                        frame
                            .extend_from_slice(&u32::try_from(output.len()).unwrap().to_be_bytes());
                        frame.extend_from_slice(output.as_bytes());
                        stream.write_all(&frame).unwrap();
                    }
                    drop(stream);
                    let (mut stream, request, _) = accept(&listener);
                    assert!(
                        request.contains("/exec/exec-id/json"),
                        "unexpected request: {request}"
                    );
                    respond(
                        &mut stream,
                        "200 OK",
                        &format!(r#"{{"Running":false,"ExitCode":{exit}}}"#),
                    );
                }
            }
        }
        finished.recv().unwrap();
        listener.set_nonblocking(true).unwrap();
        if let Ok((mut stream, _)) = listener.accept() {
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut line = Vec::new();
            let mut byte = [0];
            while !line.ends_with(b"\r\n") && stream.read_exact(&mut byte).is_ok() {
                line.push(byte[0]);
            }
            panic!(
                "unexpected request: {}",
                String::from_utf8_lossy(&line).trim_end()
            );
        }
    });
    let docker = Docker::connect_with_http(
        &format!("http://{address}"),
        5,
        bollard::API_DEFAULT_VERSION,
    )
    .unwrap();
    (docker, MockDocker { handle, done })
}
