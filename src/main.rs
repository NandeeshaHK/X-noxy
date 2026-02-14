use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::error::Error;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message;

use tokio_tungstenite::tungstenite::client::IntoClientRequest;

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

    let connect_url = format!(
        "{}/register?appname={}",
        config.relay_server, config.app_name
    );

    println!("Connecting to {}...", connect_url);

    loop {
        let mut request = match connect_url.clone().into_client_request() {
            Ok(req) => req,
            Err(e) => {
                eprintln!("Failed to create request: {}", e);
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                continue;
            }
        };

        let _ = request
            .headers_mut()
            .insert("x-api-key", config.api_key.parse().unwrap());
        let _ = request
            .headers_mut()
            .insert("appname", config.app_name.parse().unwrap());

        match connect_async(request).await {
            Ok((ws_stream, _)) => {
                println!("Connected to Relay Server!");
                let (mut write, mut read) = ws_stream.split();
                let forwarder = std::sync::Arc::new(Forwarder::new(config.local_target.clone()));
                // Create a channel to send messages from worker tasks to the WebSocket writer
                let (tx, mut rx) = tokio::sync::mpsc::channel::<Message>(100);

                // Writer task
                let write_task = tokio::spawn(async move {
                    while let Some(msg) = rx.recv().await {
                        if let Err(e) = write.send(msg).await {
                            eprintln!("Failed to send message: {}", e);
                            break;
                        }
                    }
                });

                // Heartbeat task
                let tx_heartbeat = tx.clone();
                let heartbeat_task = tokio::spawn(async move {
                    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
                    loop {
                        interval.tick().await;
                        if let Err(_) = tx_heartbeat.send(Message::Ping(vec![])).await {
                            break;
                        }
                    }
                });

                // Reader loop
                while let Some(msg) = read.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            match serde_json::from_str::<Request>(&text) {
                                Ok(req) => {
                                    println!(
                                        "Received Request: [{}] {} {}",
                                        req.id, req.method, req.path
                                    );
                                    let forwarder = forwarder.clone();
                                    let tx = tx.clone();
                                    tokio::spawn(async move {
                                        match forwarder.forward(req.clone()).await {
                                            Ok((status, mut stream)) => {
                                                let mut body_bytes = Vec::new();
                                                while let Some(chunk_res) = stream.next().await {
                                                    if let Ok(chunk) = chunk_res {
                                                        body_bytes.extend_from_slice(&chunk);
                                                    }
                                                }

                                                // Try to parse as JSON, otherwise send as string
                                                let body_json = match serde_json::from_slice::<
                                                    serde_json::Value,
                                                >(
                                                    &body_bytes
                                                ) {
                                                    Ok(v) => {
                                                        println!(
                                                            "Local Service Response Body: {}",
                                                            v
                                                        );
                                                        Some(v)
                                                    }
                                                    Err(_) => {
                                                        // Fallback: try to convert to UTF-8 string
                                                        match String::from_utf8(body_bytes) {
                                                            Ok(s) => {
                                                                println!("Local Service Response Body (String): {}", s);
                                                                Some(serde_json::Value::String(s))
                                                            }
                                                            Err(_) => {
                                                                println!("Local Service Response Body: [Binary Data]");
                                                                None
                                                            }
                                                        }
                                                    }
                                                };

                                                let resp = ResponseMessage::Response {
                                                    request_id: req.id.clone(),
                                                    status_code: status,
                                                    headers: std::collections::HashMap::new(),
                                                    body: body_json,
                                                };

                                                let json = serde_json::to_string(&resp).unwrap();
                                                match tx.send(Message::Text(json)).await {
                                                    Ok(_) => println!(
                                                        "Sent Response for Request [{}] Status: {}",
                                                        req.id, status
                                                    ),
                                                    Err(e) => {
                                                        eprintln!("Failed to send response: {}", e)
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!("Forwarding error: {}", e);
                                                // Send error response to relay
                                                let resp = ResponseMessage::Response {
                                                    request_id: req.id.clone(),
                                                    status_code: 502,
                                                    headers: std::collections::HashMap::new(),
                                                    body: Some(
                                                        serde_json::json!({ "error": e.to_string() }),
                                                    ),
                                                };
                                                if let Ok(json) = serde_json::to_string(&resp) {
                                                    let _ = tx.send(Message::Text(json)).await;
                                                }
                                            }
                                        }
                                    });
                                }
                                Err(e) => {
                                    eprintln!(
                                        "Failed to deserialize request: {}. Message: {}",
                                        e, text
                                    );
                                }
                            }
                        }
                        Ok(Message::Close(frame)) => {
                            if let Some(cf) = frame {
                                println!("Server closed connection: {} ({})", cf.code, cf.reason);
                            } else {
                                println!("Server closed connection without reason.");
                            }
                            break;
                        }
                        Ok(Message::Ping(v)) => {
                            let _ = tx.send(Message::Pong(v)).await;
                        }
                        Err(e) => {
                            eprintln!("Error reading message: {}", e);
                            break;
                        }
                        _ => {}
                    }
                }

                // Cleanup
                write_task.abort();
                heartbeat_task.abort();
                println!("Disconnected. Reconnecting in 5s...");
            }
            Err(e) => {
                eprintln!("Connection failed: {}. Retrying in 5s...", e);
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}
