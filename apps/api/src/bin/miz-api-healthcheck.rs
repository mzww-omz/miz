use std::{
    io::{Read, Write},
    net::TcpStream,
};

fn main() {
    let mut stream = TcpStream::connect("127.0.0.1:8080").expect("API is unreachable");
    stream
        .write_all(b"GET /readyz HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .expect("health request failed");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("health response failed");
    assert!(response.starts_with("HTTP/1.0 200") || response.starts_with("HTTP/1.1 200"));
}
