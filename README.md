# Pws
Persistent Websocket connection for tokio-tungstenite.

## Example
See the full example in [examples/echo.rs](examples/echo.rs).

```rust
let (tx, mut rx) =
    connect_persistent_websocket_async(Url::from_str("wss://echo.websocket.org").unwrap())
        .await
        .unwrap();
tokio::join!(
    async move {
        tx.send(Message::Text("hello".to_owned())).await.unwrap();
        // Simulate a connection close.
        tx.send(Message::Close(None)).await.unwrap();
        tx.send(Message::Text("hello again".to_owned()))
            .await
            .unwrap();
    },
    async move {
        while let Ok(msg) = rx.recv().await {
            match msg {
                Message::Text(msg) => {
                    println!("received: {msg}");
                }
                Message::ConnectionOpened => {
                    println!("connection opened");
                }
                Message::ConnectionClosed => {
                    println!("connection closed");
                }
                _ => {}
            }
        }
    }
);
```
