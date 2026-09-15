#![cfg(feature = "collector")]
use afterglow_telemetry::{connection::BATCH, websocket};
use std::net::{TcpListener, TcpStream};
use tungstenite::Message;

fn pair() -> (websocket::Socket, tungstenite::WebSocket<TcpStream>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || websocket::accept(listener.accept().unwrap().0).unwrap());
    let (client, _) = tungstenite::client(format!("ws://{address}/"), TcpStream::connect(address).unwrap()).unwrap();
    (server.join().unwrap(), client)
}
#[test]
fn messages_are_binary_bounded_and_do_not_use_tcp_length_prefixes() {
    let (mut server, mut client) = pair();
    websocket::send(&mut server, BATCH, &[1,2,3], 4).unwrap();
    assert_eq!(client.read().unwrap().into_data(), [BATCH,1,2,3]);
    assert!(websocket::send(&mut server, BATCH, &[1,2,3],3).is_err());
    client.send(Message::Binary(vec![BATCH,9])).unwrap();
    assert_eq!(websocket::receive(&mut server).unwrap().unwrap(),[BATCH,9]);
}
#[test]
fn empty_text_and_oversized_messages_are_rejected() {
    for message in [Message::Binary(vec![]), Message::Text("eval".into()), Message::Binary(vec![0;65_537])] {
        let (mut server, mut client) = pair();
        client.send(message).unwrap();
        assert!(websocket::receive(&mut server).is_err());
    }
}
#[test]
fn an_unrelated_web_origin_cannot_submit_capture_data() {
    use tungstenite::client::IntoClientRequest;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || websocket::accept(listener.accept().unwrap().0).is_err());
    let mut request = format!("ws://{address}/").into_client_request().unwrap();
    request.headers_mut().insert("origin", "https://example.invalid".parse().unwrap());
    assert!(tungstenite::client(request,TcpStream::connect(address).unwrap()).is_err());
    assert!(server.join().unwrap());
}
