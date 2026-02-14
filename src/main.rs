use base64::{engine::general_purpose, Engine as _};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::error::Error;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message;

mod forwarder;
mod protocol;

use forwarder::Forwarder;
use protocol::{Request, ResponseMessage};

#[derive(Deserialize)]
struct Config {
    app_name: String,
    relay_server: String,
    api_key: String,
    local_target: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config_content = std::fs::read_to_string("config.yaml")?;
    let config: Config = serde_yml::from_str(&config_content)?;

    let connect_url = format!("{}/register", config.relay_server);
    let url = url::Url::parse(&connect_url)?;

    let mut request =
        tokio_tungstenite::tungstenite::handshake::client::Request::get(url.to_string()).body(())?;

    request
        .headers_mut()
        .insert("x-api-key", config.api_key.parse()?);
    request
        .headers_mut()
        .insert("appname", config.app_name.parse()?);

    println!("Connecting to {}...", connect_url);

    let (ws_stream, _) = connect_async(request).await?;
    println!("Connected to Relay Server!");

    let (mut write, mut read) = ws_stream.split();
    let forwarder = std::sync::Arc::new(Forwarder::new(config.local_target.clone()));

    // Create a channel to send messages from worker tasks to the WebSocket writer
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ResponseMessage>(100);

    // Spawn a task to handle outgoing WebSocket messages
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let json = serde_json::to_string(&msg).unwrap();
            if let Err(e) = write.send(Message::Text(json)).await {
                eprintln!("Failed to send message: {}", e);
                break;
            }
        }
    });

    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Error reading message: {}", e);
                break;
            }
        };

        if let Message::Text(text) = msg {
            if let Ok(req) = serde_json::from_str::<Request>(&text) {
                let forwarder = forwarder.clone();
                let tx = tx.clone();

                tokio::spawn(async move {
                    match forwarder.forward(req.clone()).await {
                        Ok((status, mut stream)) => {
                            while let Some(chunk_res) = stream.next().await {
                                match chunk_res {
                                    Ok(chunk) => {
                                        let encoded = general_purpose::STANDARD.encode(&chunk);
                                        let _ = tx
                                            .send(ResponseMessage::StreamChunk {
                                                request_id: req.id.clone(),
                                                chunk: encoded,
                                            })
                                            .await;
                                    }
                                    Err(e) => eprintln!("Stream error: {}", e),
                                }
                            }
                            let _ = tx
                                .send(ResponseMessage::StreamEnd {
                                    request_id: req.id.clone(),
                                    status_code: status,
                                })
                                .await;
                        }
                        Err(e) => eprintln!("Forwarding error: {}", e),
                    }
                });
            }
        }
    }

    Ok(())
}
